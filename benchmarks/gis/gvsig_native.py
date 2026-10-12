"""Jython native JTS validation, using gvSIG's bundled geometry plugin."""

from java.util import ArrayList
from org.locationtech.jts import JTSVersion as JTSVersion
from org.locationtech.jts.geom import GeometryFactory
from org.locationtech.jts.index.strtree import STRtree
from org.locationtech.jts.io import WKTReader
from org.locationtech.jts.operation.union import UnaryUnionOp


def load_sources(files, wkt):
    """Parse input once with gvSIG's native JTS implementation."""
    import json

    reader = WKTReader()
    result = []
    with open(files["input"]) as stream:
        features = json.load(stream)["features"]
    for feature in features:
        result.append(
            (
                int(feature["properties"]["bench_id"]),
                reader.read(wkt(feature["geometry"])),
            )
        )
    return result


def validate(rule, geometries):
    """Index native geometries and materialize native violation geometries."""
    index = STRtree()
    for fid, geometry in geometries:
        index.insert(geometry.getEnvelopeInternal(), (fid, geometry))
    index.build()
    errors = []
    if rule == "polygon/must-not-have-gaps":
        parts = ArrayList()
        for _fid, geometry in geometries:
            parts.add(geometry)
        merged = UnaryUnionOp.union(parts)
        factory = GeometryFactory()
        for n in range(merged.getNumGeometries()):
            polygon = merged.getGeometryN(n)
            for r in range(polygon.getNumInteriorRing()):
                ring = factory.createLinearRing(
                    polygon.getInteriorRingN(r).getCoordinates()
                )
                void = factory.createPolygon(ring).difference(merged)
                for p in range(void.getNumGeometries()):
                    part = void.getGeometryN(p)
                    if part.getArea() > 0:
                        errors.append((None, None, part))
        return errors
    for fid, geometry in geometries:
        if rule == "line/must-not-have-dangles":
            if geometry.isClosed():
                continue
            for endpoint in (geometry.getStartPoint(), geometry.getEndPoint()):
                uses = 0
                for _other_id, other in index.query(endpoint.getEnvelopeInternal()):
                    if other.intersects(endpoint):
                        uses += (
                            2
                            if other.isClosed()
                            or not (
                                other.getStartPoint().equalsTopo(endpoint)
                                or other.getEndPoint().equalsTopo(endpoint)
                            )
                            else 1
                        )
                if uses == 1:
                    errors.append((fid, None, endpoint))
            continue
        for other_id, other in index.query(geometry.getEnvelopeInternal()):
            if other_id <= fid:
                continue
            if rule == "point/must-not-overlap":
                if geometry.equalsTopo(other):
                    errors.append((fid, other_id, geometry.copy()))
            else:
                intersection = geometry.intersection(other)
                if intersection.getArea() > 0:
                    errors.append((fid, other_id, intersection))
    return errors
