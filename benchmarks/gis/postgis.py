"""PostGIS SQL workflows with server-side timing and fresh spatial indexes."""

import json
import os
from pathlib import Path
from typing import Any, LiteralString, cast

import psycopg
from psycopg import sql

from benchmarks.gis.fixtures import ROOT
from benchmarks.gis.sessions import EngineFailure

from .catalog import SUPPORT


def _query(rule: str) -> str:
    """Build a fixed SQL workflow materializing violation geometries and IDs."""

    def missing(predicate: str, kind: str = "not-covered") -> str:
        return (
            f"SELECT '{kind}'::text kind, a.id, NULL::bigint destination, "
            f"0::int layer, a.geom FROM a WHERE NOT EXISTS "
            f"(SELECT 1 FROM b WHERE a.geom && b.geom AND {predicate})"
        )

    if rule == "point/must-be-inside":
        return missing("ST_ContainsProperly(b.geom, a.geom)", "outside")
    if rule == "polygon/must-contain-point":
        return missing("ST_ContainsProperly(a.geom, b.geom)", "missing-point")
    if rule == "point/must-be-covered-by-lines":
        return missing("ST_Covers(b.geom, a.geom)")
    if rule == "point/must-be-covered-by-polygon-boundaries":
        return missing("ST_Covers(ST_Boundary(b.geom), a.geom)")
    if rule == "point/must-be-covered-by-line-endpoints":
        return missing("ST_Covers(ST_Boundary(b.geom), a.geom)")
    if rule == "line/endpoints-must-be-covered-by-points":
        return (
            "SELECT 'not-covered'::text kind,a.id,NULL::bigint destination,0::int layer,"
            "e.geom FROM a CROSS JOIN LATERAL ST_Dump(a.geom) p "
            "CROSS JOIN LATERAL ST_Dump(ST_Boundary(p.geom)) e "
            "WHERE NOT EXISTS (SELECT 1 FROM b WHERE b.geom && e.geom "
            "AND ST_Covers(b.geom,e.geom))"
        )
    if rule == "geometry/must-not-be-multipart":
        return (
            "SELECT 'multipart'::text kind,id,NULL::bigint destination,0::int layer,"
            "geom FROM a WHERE ST_NumGeometries(geom)>1"
        )
    if rule == "polygon/must-not-have-gaps":
        return (
            "WITH merged AS (SELECT ST_UnaryUnion(ST_Collect(geom)) geom FROM a),"
            "polygons AS (SELECT (ST_Dump(geom)).geom FROM merged) "
            "SELECT 'gap'::text kind,NULL::bigint id,NULL::bigint destination,"
            "0::int layer,d.geom "
            "FROM polygons CROSS JOIN LATERAL generate_series(1,ST_NumInteriorRings(geom)) n "
            "CROSS JOIN merged m CROSS JOIN LATERAL ST_Dump(ST_Difference("
            "ST_MakePolygon(ST_InteriorRingN(polygons.geom,n)),m.geom)) d"
        )
    if rule == "line/must-be-inside":
        return (
            "WITH cover AS (SELECT ST_UnaryUnion(ST_Collect(geom)) geom FROM b) "
            "SELECT 'outside'::text kind,a.id,NULL::bigint destination,0::int layer,"
            "d.geom FROM a CROSS JOIN cover CROSS JOIN LATERAL ST_Dump("
            "ST_CollectionExtract(ST_Difference(a.geom,cover.geom),2)) d "
            "WHERE ST_Length(d.geom)>0"
        )
    if rule in {"line/must-not-have-dangles", "line/must-not-have-pseudo-nodes"}:
        # Interior uses contribute two; closed parts have no endpoints.
        cte = (
            "WITH parts AS (SELECT id,(ST_Dump(geom)).geom FROM a), "
            "ends AS (SELECT id,(ST_Dump(ST_Boundary(geom))).geom FROM parts), "
            "degree AS (SELECT e.id,e.geom,(SELECT SUM(CASE WHEN ST_IsClosed(p.geom) "
            "THEN 2 WHEN ST_Equals(ST_StartPoint(p.geom),e.geom) OR "
            "ST_Equals(ST_EndPoint(p.geom),e.geom) THEN CASE WHEN c.id=e.id AND EXISTS "
            "(SELECT 1 FROM ST_DumpSegments(p.geom) s WHERE ST_Length(s.geom)>0 "
            "AND NOT (s.path[1]=1 AND ST_Equals(ST_StartPoint(p.geom),e.geom)) "
            "AND NOT (s.path[1]=ST_NPoints(p.geom)-1 AND ST_Equals(ST_EndPoint(p.geom),e.geom)) "
            "AND ST_Intersects(s.geom,e.geom)) THEN 3 ELSE 1 END ELSE 2 END) "
            "FROM a c CROSS JOIN LATERAL ST_Dump(c.geom) p "
            "WHERE c.geom && e.geom AND ST_Intersects(p.geom,e.geom)) uses FROM ends e) "
        )
        if rule.endswith("dangles"):
            return cte + (
                "SELECT 'dangle'::text kind,id,NULL::bigint destination,0::int layer,"
                "geom FROM degree WHERE uses=1"
            )
        return cte + (
            "SELECT 'pseudo-node'::text kind,MIN(id) id,MAX(id) destination,0::int layer,"
            "geom FROM degree WHERE uses=2 GROUP BY geom HAVING COUNT(DISTINCT id)=2"
        )
    if rule not in SUPPORT["postgis"]:
        raise ValueError(f"unsupported PostGIS rule: {rule}")
    other = "b" if rule.endswith("-with") else "a"
    pairs = "a.id<c.id AND " if other == "a" else ""
    if rule == "point/must-not-overlap":
        predicate, kind = "ST_Equals(a.geom,c.geom)", "overlap"
    elif rule.startswith("polygon/"):
        predicate, kind = "ST_Area(ST_Intersection(a.geom,c.geom))>0", "overlap"
    elif "overlap" in rule:
        predicate, kind = (
            "ST_Length(ST_CollectionExtract(ST_Intersection(a.geom,c.geom),2))>0",
            "overlap",
        )
    else:
        predicate, kind = "ST_Relate(a.geom,c.geom,'T********')", "crossing"
    return (
        f"SELECT '{kind}'::text kind,a.id,c.id destination,0::int layer,"
        f"ST_Intersection(a.geom,c.geom) geom FROM a JOIN {other} c ON "
        f"{pairs}a.geom && c.geom WHERE {predicate}"
    )


def query(rule: str) -> LiteralString:
    """Expose SQL generated solely from the fixed rule manifest, never user SQL."""
    return cast(LiteralString, _query(rule))


class PostgisSession:
    """A dedicated connection with private, unindexed source tables."""

    def __init__(
        self, fixture: dict[str, Any], directory: Path, startup_timeout: float
    ) -> None:
        """Load geometries without spatial indexes before timing any sample."""
        self.connection = psycopg.connect(
            os.environ.get("GIS_POSTGIS_DSN", "dbname=gis_benchmark host=/tmp/gis-pg"),
            autocommit=True,
            connect_timeout=max(1, int(startup_timeout)),
        )
        self.closed = False
        try:
            with self.connection.cursor() as cursor:
                cursor.execute("SET max_parallel_workers_per_gather=0")
                cursor.execute("SELECT postgis_version(),postgis_full_version()")
                version = cursor.fetchone()
                assert version is not None
                self.metadata = {
                    "version": version[0].split()[0],
                    "libraries": version[1],
                    "workflow": "SQL",
                }
                for name in ("input", "other"):
                    cursor.execute(
                        f"CREATE TEMP TABLE source_{name}(id bigint,geom geometry)"
                    )
                    if name not in fixture:
                        continue
                    features = json.loads((ROOT / fixture[name]["path"]).read_text())[
                        "features"
                    ]
                    cursor.executemany(
                        f"INSERT INTO source_{name} VALUES (%s,"
                        "ST_SetSRID(ST_GeomFromGeoJSON(%s),3857))",
                        [
                            (f["properties"]["bench_id"], json.dumps(f["geometry"]))
                            for f in features
                        ],
                    )
        except (psycopg.Error, OSError, ValueError):
            self.close()
            raise

    def sample(self, rule: str, timeout: float) -> dict[str, Any]:
        """Time spatial indexing, statistics and full error-geometry materialization."""
        with self.connection.cursor() as cursor:
            cursor.execute(
                sql.SQL("SET statement_timeout={}").format(
                    sql.Literal(int(timeout * 1000))
                )
            )
            for table in ("a", "b", "violations"):
                cursor.execute(f"DROP TABLE IF EXISTS {table}")
            cursor.execute("CREATE TEMP TABLE a AS SELECT * FROM source_input")
            cursor.execute("CREATE TEMP TABLE b AS SELECT * FROM source_other")
            try:
                cursor.execute(
                    sql.SQL(
                        "DO $benchmark$ DECLARE started timestamptz; BEGIN "
                        "started := clock_timestamp(); "
                        "CREATE INDEX ON a USING gist(geom); CREATE INDEX ON b USING gist(geom); "
                        "ANALYZE a; ANALYZE b; CREATE TEMP TABLE violations AS "
                        + query(rule)
                        + "; "
                        "PERFORM set_config('benchmark.seconds', "
                        "EXTRACT(epoch FROM clock_timestamp()-started)::text,false); "
                        "END $benchmark$"
                    )
                )
            except psycopg.errors.QueryCanceled as error:
                raise EngineFailure(
                    "timeout", "PostGIS statement deadline exceeded"
                ) from error
            cursor.execute(
                "SELECT current_setting('benchmark.seconds')::double precision"
            )
            duration = cursor.fetchone()
            assert duration is not None
            seconds = duration[0]
            cursor.execute(
                "SELECT kind,id,destination,layer,ST_AsGeoJSON(geom,17)::json FROM violations ORDER BY id,destination"
            )
            errors = [
                dict(
                    zip(
                        ("kind", "id", "destination", "layer", "geometry"),
                        row,
                        strict=True,
                    )
                )
                for row in cursor.fetchall()
            ]
        return {
            "status": "ok",
            "seconds": seconds,
            "raw_count": len(errors),
            "errors": errors,
        }

    def close(self) -> None:
        """Release temporary data by closing the benchmark connection."""
        if not self.closed:
            self.closed = True
            self.connection.close()
