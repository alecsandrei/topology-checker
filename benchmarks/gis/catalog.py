"""Explicit benchmark capabilities, separate from public product metadata."""

from benchmarks.qgis.fixtures import MAPPINGS

ENGINES = ("qgis", "grass", "postgis", "gvsig")
SHARED_RULES = ("point/must-be-inside", "polygon/must-contain-point")
LABELS = {
    "rust": "topology-checker",
    "qgis": "QGIS",
    "grass": "GRASS GIS",
    "postgis": "PostGIS SQL",
    "gvsig": "gvSIG",
    "fme": "FME",
}
GVSIG = {
    "point/must-not-overlap": "MustBeDisjointPoint",
    "point/must-be-inside": "MustBeProperlyInsidePolygonsPoint",
    "line/must-not-have-dangles": "MustNotHaveDanglesLine",
    "line/must-not-overlap-with": "MustNotOverlapWithLine",
    "line/must-be-inside": "MustBeInsideLine",
    "line/must-not-intersect": "MustNotIntersectLine",
    "line/must-not-intersect-with": "MustNotIntersectWithLine",
    "polygon/must-not-overlap": "MustNotOverlapPolygon",
    "polygon/must-not-have-gaps": "MustNotHaveGapsPolygon",
    "point/must-be-covered-by-lines": "PointsMustBeCoveredByLinePoint",
    "point/must-be-covered-by-line-endpoints": "MustBeCoveredByEndpointOfPoint",
    "point/must-be-covered-by-polygon-boundaries": "MustBeCoveredByBoundaryOfPoint",
    "polygon/must-contain-point": "ContainsPointPolygon",
}
GRASS = {
    "point/must-be-inside": "within, reverse selection",
    "point/must-be-covered-by-lines": "intersects, reverse selection",
    "point/must-be-covered-by-polygons": "intersects, reverse selection",
    "point/must-not-be-inside-polygons": "within",
    "point/must-not-intersect-lines": "intersects",
    "point/must-not-intersect-polygons": "intersects",
    "polygon/must-contain-point": "contains, reverse selection",
    "polygon/must-not-contain-points": "contains",
    "point/must-not-overlap": "v.clean rmdupl (repair workflow)",
    "line/must-not-have-dangles": "v.clean rmdangle (repair workflow)",
}
RELATED = {
    ("grass", "point/must-not-overlap"),
    ("grass", "line/must-not-have-dangles"),
}
SUPPORT = {
    "qgis": {rule: mapping[0] for rule, mapping in MAPPINGS.items()},
    "gvsig": GVSIG,
    "grass": GRASS,
    "postgis": {rule: "SQL workflow" for rule in sorted(set(MAPPINGS) | set(GVSIG))},
    "fme": {rule: "SpatialFilter workflow" for rule in SHARED_RULES},
}
