"""Explicit native benchmark capabilities, separate from product metadata."""

from benchmarks.qgis.fixtures import MAPPINGS

ENGINES = ("qgis", "grass", "postgis", "gvsig")
CONTAINMENT_RULES = ("point/must-be-inside", "polygon/must-contain-point")
HEADLINE_RULES = (
    *CONTAINMENT_RULES,
    "point/must-not-overlap",
    "line/must-not-have-dangles",
    "polygon/must-not-overlap",
    "polygon/must-not-have-gaps",
)
LABELS = {
    "rust": "topology-checker",
    "qgis": "QGIS",
    "grass": "GRASS GIS",
    "postgis": "PostGIS SQL",
    "gvsig": "gvSIG",
}
CHECK_LABELS = (
    "Points inside\npolygons",
    "Polygons contain\na point",
    "Duplicate\npoints",
    "Line\ndangles",
    "Polygon\noverlaps",
    "Polygon\ngaps",
)
GVSIG = {
    "point/must-be-inside": "PointMustBeProperlyInsidePolygon",
    "line/must-not-overlap-with": "MustNotOverlapWithLine",
    "line/must-be-inside": "MustBeInsideLine",
    "line/must-not-intersect": "MustNotIntersectLine",
    "line/must-not-intersect-with": "MustNotIntersectWithLine",
    "point/must-be-covered-by-lines": "PointsMustBeCoveredByLinePoint",
    "point/must-be-covered-by-line-endpoints": "MustBeCoveredByEndpointOfPoint",
    "point/must-be-covered-by-polygon-boundaries": "MustBeCoveredByBoundaryOfPoint",
    "polygon/must-contain-point": "PolygonContainsPoint",
}
NATIVE_RULES = HEADLINE_RULES[2:]
NATIVE_GVSIG = {
    NATIVE_RULES[0]: "native STRtree and JTS equalsTopo pairs",
    NATIVE_RULES[1]: "native STRtree and JTS endpoint incidence",
    NATIVE_RULES[2]: "native STRtree and JTS positive-area intersection pairs",
    NATIVE_RULES[3]: "native JTS unary union and bounded void difference",
}
GVSIG.update(NATIVE_GVSIG)
GRASS = {
    "point/must-be-inside": "v.select within, reverse selection",
    "point/must-be-covered-by-lines": "v.select intersects, reverse selection",
    "point/must-be-covered-by-polygons": "v.select intersects, reverse selection",
    "point/must-not-be-inside-polygons": "v.select within",
    "point/must-not-intersect-lines": "v.select intersects",
    "point/must-not-intersect-polygons": "v.select intersects",
    "polygon/must-contain-point": "v.select contains, reverse selection",
    "polygon/must-not-contain-points": "v.select contains",
    NATIVE_RULES[
        0
    ]: "v.to.db coordinates, SQLite duplicate-pair query, native error vectors",
    NATIVE_RULES[1]: "v.clean break, native node degree and original categories",
    NATIVE_RULES[2]: "native spatial index and attributed v.overlay intersection pairs",
    NATIVE_RULES[
        3
    ]: "v.clean break,rmdupl,bpol and original-ring coverage of bounded native faces",
}
SUPPORT = {
    "qgis": {rule: mapping[0] for rule, mapping in MAPPINGS.items()},
    "gvsig": GVSIG,
    "grass": GRASS,
    "postgis": {rule: "SQL workflow" for rule in sorted(set(MAPPINGS) | set(GVSIG))},
}
SUPPORT["qgis"].update(
    dict(
        zip(
            NATIVE_RULES,
            (
                "QgsSpatialIndex and native geometry equality pairs",
                "QgsSpatialIndex and endpoint incidence",
                "QgsSpatialIndex and positive-area intersection pairs",
                "QgsGeometry.unaryUnion and bounded void difference",
            ),
            strict=True,
        )
    )
)
SUPPORT["postgis"].update(
    dict(
        zip(
            HEADLINE_RULES,
            (
                "GiST and ST_ContainsProperly reverse selection",
                "GiST and ST_ContainsProperly reverse selection",
                "GiST and ST_Equals pairs",
                "GiST and endpoint-incidence SQL",
                "GiST and positive-area ST_Intersection pairs",
                "ST_UnaryUnion and bounded void ST_Difference",
            ),
            strict=True,
        )
    )
)
