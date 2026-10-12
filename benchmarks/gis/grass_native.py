"""Native GRASS validation with original categories and bounded topology faces."""

import ctypes
import sqlite3
from string import Template

import grass.lib.vector as libvect
import grass.script as gs
from grass.pygrass.vector import Vector, VectorTopo
from grass.pygrass.vector.basic import Bbox
from grass.pygrass.vector.geometry import Boundary, Centroid, Point


def coverage_templates(features: list[dict]) -> dict:
    """Preload original native rings and an unindexed, attributed boundary map."""
    rings = {}
    index = VectorTopo("source_index")
    index.open("w")
    for feature in features:
        fid = int(feature["properties"]["bench_id"])
        rings[fid] = [
            Boundary(points=ring) for ring in feature["geometry"]["coordinates"]
        ]
        index.write(rings[fid][0], cat=fid)
    index.close()
    return rings


def polygon_templates(features: list[dict]) -> list[str]:
    """Load original polygons individually so overlays retain equal and contained IDs."""
    names = []
    index = VectorTopo("source_index")
    index.open("w")
    for feature in features:
        fid = int(feature["properties"]["bench_id"])
        name = f"source_poly_{fid}"
        vector = VectorTopo(name)
        vector.open("w")
        for ring in feature["geometry"]["coordinates"]:
            vector.write(Boundary(points=ring), cat=fid)
        index.write(Boundary(points=feature["geometry"]["coordinates"][0]), cat=fid)
        vector.close()
        vector.open("rw")
        left, right = ctypes.c_int(), ctypes.c_int()
        libvect.Vect_get_line_areas(
            vector.c_mapinfo, 1, ctypes.byref(left), ctypes.byref(right)
        )
        area = max(left.value, right.value)
        x, y = ctypes.c_double(), ctypes.c_double()
        if (
            libvect.Vect_get_point_in_area(
                vector.c_mapinfo, area, ctypes.byref(x), ctypes.byref(y)
            )
            < 0
        ):
            raise ValueError("Cannot find native polygon interior")
        vector.write(Centroid(x=x.value, y=y.value), cat=fid)
        vector.close()
        names.append(name)
    index.close()
    return names


def build(name: str) -> None:
    """Build topology and spatial indexes natively without a separate process."""
    vector = Vector(name)
    vector.open("rw")
    vector.build()
    vector.close()


def duplicates() -> list[dict]:
    """Extract native coordinates, query every pair and write native error points."""
    gs.run_command(
        "v.db.addcolumn",
        map="input",
        columns="bench_x double precision,bench_y double precision",
        quiet=True,
    )
    gs.run_command(
        "v.to.db",
        map="input",
        type="point",
        option="coor",
        columns="bench_x,bench_y",
        overwrite=True,
        quiet=True,
    )
    connection = gs.vector_db("input")[1]
    with sqlite3.connect(
        Template(connection["database"]).substitute(gs.gisenv())
    ) as database:
        table = connection["table"]
        database.execute(
            f'CREATE INDEX IF NOT EXISTS duplicate_xy ON "{table}" (bench_x,bench_y)'
        )
        pairs = database.execute(
            f'SELECT a.bench_id,b.bench_id,a.bench_x,a.bench_y FROM "{table}" a JOIN "{table}" b ON a.bench_x=b.bench_x AND a.bench_y=b.bench_y AND a.bench_id<b.bench_id'
        ).fetchall()
    errors = VectorTopo("errors")
    errors.open("w", overwrite=True)
    identities = []
    for fid, destination, x, y in pairs:
        errors.write(Point(x, y), cat=len(identities) + 1)
        identities.append(
            {"kind": "overlap", "id": fid, "destination": destination, "layer": 0}
        )
    errors.close()
    return identities


def dangles() -> list[dict]:
    """Node linework without deleting it, then inspect degree-one native nodes."""
    gs.run_command(
        "v.clean",
        input="input",
        output="noded",
        tool="break",
        threshold=0,
        overwrite=True,
        quiet=True,
    )
    vector = VectorTopo("noded")
    vector.open("r")
    output = VectorTopo("errors")
    output.open("w", overwrite=True)
    identities = []
    for node in vector.viter("nodes"):
        if node.nlines == 1:
            edge = vector.read(abs(next(node.ilines())))
            fid = edge.cat
            output.write(Point(*node.coords()), cat=len(identities) + 1)
            identities.append({"kind": "dangle", "id": fid, "layer": 0})
    output.close()
    vector.close()
    return identities


def overlaps(names: list[str]) -> list[dict]:
    """Select candidates natively and overlay original, attributed polygon features."""
    build("index")
    for name in names:
        build(name.replace("source_", ""))
    index = VectorTopo("index")
    index.open("r")
    records = []
    pairs = set()
    for boundary in index.viter("boundaries"):
        fid = boundary.cat
        for candidate in (
            index.find_by_bbox.geos(boundary.bbox(), type="boundary") or []
        ):
            destination = candidate.cat
            if destination > fid:
                pairs.add((fid, destination))
    index.close()
    for fid, destination in sorted(pairs):
        gs.run_command(
            "v.overlay",
            ainput=f"poly_{fid}",
            binput=f"poly_{destination}",
            operator="and",
            output=f"overlap_{fid}_{destination}",
            snap=0,
            overwrite=True,
            quiet=True,
        )
        vector = VectorTopo(f"overlap_{fid}_{destination}")
        vector.open("r")
        for area in vector.viter("areas"):
            if area.cat is not None and area.area() > 0:
                # The overlay itself is the native, materialized error vector.
                # Keep every pair output until extraction after timing.
                records.append(
                    {
                        "kind": "overlap",
                        "id": fid,
                        "destination": destination,
                        "layer": 0,
                        "native_map": vector.name,
                        "native_area": area.id,
                    }
                )
        vector.close()
    return records


def gaps(rings: dict) -> list[dict]:
    """Extract bounded uncovered faces, retaining any covered islands as holes."""
    gs.run_command(
        "v.clean",
        input="input",
        output="faces",
        tool="break,rmdupl,bpol",
        threshold="0,0,0",
        overwrite=True,
        quiet=True,
    )
    build("index")
    index = VectorTopo("index")
    index.open("r")
    vector = VectorTopo("faces")
    vector.open("r")
    records = []
    for area in vector.viter("areas"):
        if area.centroid() is None and area.area() > 0:
            x, y = ctypes.c_double(), ctypes.c_double()
            if (
                libvect.Vect_get_point_in_area(
                    vector.c_mapinfo, area.id, ctypes.byref(x), ctypes.byref(y)
                )
                < 0
            ):
                raise ValueError("Cannot find native face interior")
            box = Bbox(north=y.value, south=y.value, east=x.value, west=x.value)
            covered = False
            for candidate in index.find_by_bbox.geos(box, type="boundary") or []:
                original = rings[candidate.cat]
                if libvect.Vect_point_in_poly(
                    x.value, y.value, original[0].c_points
                ) > 0 and not any(
                    libvect.Vect_point_in_poly(x.value, y.value, ring.c_points) > 0
                    for ring in original[1:]
                ):
                    covered = True
                    break
            if covered:
                continue
            records.append(
                {
                    "kind": "gap",
                    "id": None,
                    "layer": 0,
                    "native_map": vector.name,
                    "native_area": area.id,
                }
            )
    vector.close()
    index.close()
    return records


def extract(records: list[dict]) -> list[dict]:
    """Convert native error snapshots to WKT only after timing."""
    # Native maps already hold materialized errors. Read and serialize after timing.
    for record in records:
        if "native_map" in record:
            vector = VectorTopo(record.pop("native_map"))
            vector.open("r")
            from grass.pygrass.vector.geometry import Area

            area = Area(v_id=record.pop("native_area"), c_mapinfo=vector.c_mapinfo)
            record["geometry"] = area.to_wkt()
            vector.close()
    return records
