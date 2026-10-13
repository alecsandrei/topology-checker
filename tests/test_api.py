"""Acceptance coverage for rules, adapters, provenance, and immutable reports."""

from __future__ import annotations

import importlib.util
import json
from collections.abc import Sequence
from concurrent.futures import ThreadPoolExecutor
from dataclasses import FrozenInstanceError
from pathlib import Path
from typing import Any

import pytest
import shapely
from pyproj import CRS
from shapely.affinity import affine_transform
from shapely.geometry import (
    GeometryCollection,
    LineString,
    MultiLineString,
    MultiPoint,
    MultiPolygon,
    Point,
    Polygon,
    box,
    mapping,
)
from shapely.geometry.base import BaseGeometry
from topology_checker import (
    ErrorType,
    GeometryKind,
    Rule,
    RuleVariant,
    Topology,
    _native,
    explode_lines,
    merge_lines,
)

EDGE = LineString([(0, 0), (10, 0)])
INTERIOR = LineString([(1, 1), (2, 1)])
OUTSIDE = LineString([(-2, -1), (-1, -1)])
SQUARE = box(0, 0, 10, 10)
PERIMETER = LineString(SQUARE.exterior.coords)
CROSSING = [LineString([(0, 0), (2, 2)]), LineString([(0, 2), (2, 0)])]
TOUCHING = [LineString([(0, 0), (2, 0)]), LineString([(1, 0), (1, 1)])]


def geometry_fixture(name: str) -> list[BaseGeometry]:
    """Load the same frozen source coordinates used by the native rule tests."""
    path = Path(__file__).parents[1] / "crates/core/tests/fixtures/geometry_cases.json"
    data = json.loads(path.read_text())
    return [shapely.from_geojson(json.dumps(g)) for g in data[name]["geometries"]]


# Fixtures exercise each Rust rule through the Python WKB boundary.
INVALID_CASES: list[
    tuple[str, Rule, Sequence[BaseGeometry], Sequence[BaseGeometry] | None]
] = [
    ("point/must-not-overlap", Rule.MUST_NOT_OVERLAP, [Point(0, 0)] * 2, None),
    (
        "point/must-not-overlap-with",
        Rule.MUST_NOT_OVERLAP,
        [Point(0, 0)],
        [Point(0, 0)],
    ),
    ("point/must-be-inside", Rule.MUST_BE_INSIDE, [Point(0, 5)], [SQUARE]),
    ("line/must-not-have-dangles", Rule.MUST_NOT_HAVE_DANGLES, [EDGE], None),
    (
        "line/must-not-have-pseudo-nodes",
        Rule.MUST_NOT_HAVE_PSEUDO_NODES,
        [LineString([(0, 0), (1, 0)]), LineString([(1, 0), (2, 0)])],
        None,
    ),
    ("line/must-not-overlap", Rule.MUST_NOT_OVERLAP, [EDGE, EDGE], None),
    ("line/must-not-overlap-with", Rule.MUST_NOT_OVERLAP, [EDGE], [EDGE]),
    (
        "line/must-not-self-overlap",
        Rule.MUST_NOT_SELF_OVERLAP,
        [LineString([(0, 0), (2, 0), (0, 0)])],
        None,
    ),
    ("line/must-not-intersect", Rule.MUST_NOT_INTERSECT, CROSSING, None),
    (
        "line/must-not-intersect-with",
        Rule.MUST_NOT_INTERSECT,
        CROSSING[:1],
        CROSSING[1:],
    ),
    (
        "line/must-not-intersect-or-touch-interior",
        Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR,
        TOUCHING,
        None,
    ),
    (
        "line/must-not-intersect-or-touch-interior-with",
        Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR,
        TOUCHING[:1],
        TOUCHING[1:],
    ),
    (
        "line/must-not-self-intersect",
        Rule.MUST_NOT_SELF_INTERSECT,
        [LineString([(0, 0), (2, 2), (0, 2), (2, 0)])],
        None,
    ),
    ("line/must-be-inside", Rule.MUST_BE_INSIDE, [OUTSIDE], [SQUARE]),
    (
        "polygon/must-not-overlap",
        Rule.MUST_NOT_OVERLAP,
        [SQUARE, box(5, 0, 15, 10)],
        None,
    ),
    (
        "polygon/must-not-overlap-with",
        Rule.MUST_NOT_OVERLAP,
        [SQUARE],
        [box(5, 0, 15, 10)],
    ),
    (
        "polygon/must-not-have-gaps",
        Rule.MUST_NOT_HAVE_GAPS,
        [Polygon(SQUARE.exterior.coords, [box(3, 3, 7, 7).exterior.coords])],
        None,
    ),
    (
        "geometry/must-not-be-multipart",
        Rule.MUST_NOT_BE_MULTIPART,
        [MultiPoint([(0, 0), (1, 0)])],
        None,
    ),
    ("point/must-be-covered-by-lines", Rule.MUST_BE_COVERED_BY, [Point(5, 1)], [EDGE]),
    (
        "point/must-be-covered-by-line-endpoints",
        Rule.MUST_BE_COVERED_BY_ENDPOINTS,
        [Point(5, 0)],
        [EDGE],
    ),
    (
        "point/must-be-covered-by-polygon-boundaries",
        Rule.MUST_BE_COVERED_BY_BOUNDARIES,
        [Point(5, 5)],
        [SQUARE],
    ),
    (
        "line/endpoints-must-be-covered-by-points",
        Rule.ENDPOINTS_MUST_BE_COVERED_BY,
        [EDGE],
        [Point(0, 0)],
    ),
    (
        "line/must-be-covered-by-polygon-boundaries",
        Rule.MUST_BE_COVERED_BY_BOUNDARIES,
        [INTERIOR],
        [SQUARE],
    ),
    (
        "polygon/boundaries-must-be-covered-by-lines",
        Rule.BOUNDARIES_MUST_BE_COVERED_BY,
        [SQUARE],
        [EDGE],
    ),
    ("polygon/must-contain-point", Rule.MUST_CONTAIN, [SQUARE], [Point(0, 0)]),
    (
        "polygon/must-contain-one-point",
        Rule.MUST_CONTAIN_ONE,
        [SQUARE],
        [MultiPoint([(1, 1), (20, 20)])],
    ),
    (
        "line/must-not-cross-polygon-boundaries",
        Rule.MUST_NOT_CROSS_BOUNDARIES,
        [LineString([(-1, 5), (11, 5)])],
        [SQUARE],
    ),
    (
        "geometry/must-not-be-contained-by",
        Rule.MUST_NOT_BE_CONTAINED_BY,
        [Point(1, 1)],
        [SQUARE],
    ),
    (
        "point/must-be-covered-by-polygons",
        Rule.MUST_BE_COVERED_BY,
        [Point(20, 20)],
        [SQUARE],
    ),
    (
        "point/must-not-be-inside-polygons",
        Rule.MUST_NOT_BE_INSIDE,
        [Point(5, 5)],
        [SQUARE],
    ),
    ("point/must-not-intersect-lines", Rule.MUST_NOT_INTERSECT, [Point(0, 0)], [EDGE]),
    (
        "point/must-not-intersect-polygons",
        Rule.MUST_NOT_INTERSECT,
        [Point(0, 0)],
        [SQUARE],
    ),
    (
        "line/must-be-properly-inside-polygons",
        Rule.MUST_BE_PROPERLY_INSIDE,
        [EDGE],
        [SQUARE],
    ),
    (
        "line/must-not-overlap-with-polygons",
        Rule.MUST_NOT_OVERLAP,
        [INTERIOR],
        [SQUARE],
    ),
    (
        "line/must-not-overlap-polygon-boundaries",
        Rule.MUST_NOT_OVERLAP_BOUNDARIES,
        [EDGE],
        [SQUARE],
    ),
    ("line/must-not-intersect-polygons", Rule.MUST_NOT_INTERSECT, [INTERIOR], [SQUARE]),
    (
        "line/endpoints-must-be-covered-by-polygon-boundaries",
        Rule.ENDPOINTS_MUST_BE_COVERED_BY_BOUNDARIES,
        [INTERIOR],
        [SQUARE],
    ),
    ("polygon/must-not-contain-points", Rule.MUST_NOT_CONTAIN, [SQUARE], [Point(5, 5)]),
]

VALID_BINARY_CASES: list[tuple[Rule, BaseGeometry, BaseGeometry]] = [
    (Rule.MUST_BE_INSIDE, Point(5, 5), SQUARE),
    (Rule.MUST_BE_COVERED_BY, Point(5, 0), EDGE),
    (Rule.MUST_BE_COVERED_BY_ENDPOINTS, Point(0, 0), EDGE),
    (Rule.MUST_BE_COVERED_BY_BOUNDARIES, Point(0, 5), SQUARE),
    (
        Rule.ENDPOINTS_MUST_BE_COVERED_BY,
        EDGE,
        MultiPoint([(0, 0), (10, 0)]),
    ),
    (Rule.MUST_BE_COVERED_BY_BOUNDARIES, EDGE, SQUARE),
    (Rule.BOUNDARIES_MUST_BE_COVERED_BY, SQUARE, PERIMETER),
    (Rule.MUST_CONTAIN, SQUARE, Point(1, 1)),
    (Rule.MUST_CONTAIN_ONE, SQUARE, Point(1, 1)),
    (Rule.MUST_NOT_CROSS_BOUNDARIES, EDGE, SQUARE),
    (Rule.MUST_NOT_BE_CONTAINED_BY, Point(20, 20), SQUARE),
    (Rule.MUST_BE_COVERED_BY, Point(0, 5), SQUARE),
    (Rule.MUST_NOT_BE_INSIDE, Point(0, 5), SQUARE),
    (Rule.MUST_NOT_INTERSECT, Point(1, 1), EDGE),
    (Rule.MUST_NOT_INTERSECT, Point(20, 20), SQUARE),
    (Rule.MUST_BE_PROPERLY_INSIDE, INTERIOR, SQUARE),
    (Rule.MUST_NOT_OVERLAP, OUTSIDE, SQUARE),
    (Rule.MUST_NOT_OVERLAP_BOUNDARIES, INTERIOR, SQUARE),
    (Rule.MUST_NOT_INTERSECT, OUTSIDE, SQUARE),
    (Rule.ENDPOINTS_MUST_BE_COVERED_BY_BOUNDARIES, EDGE, SQUARE),
    (Rule.MUST_NOT_CONTAIN, SQUARE, Point(0, 0)),
    (Rule.MUST_BE_INSIDE, EDGE, SQUARE),
]


def test_enum_registry_parity() -> None:
    """Static enums retain native values, order, and metadata."""
    native = json.loads(_native.registry())
    assert len(Rule) == 24
    variants = [variant for rule in Rule for variant in rule.variants]
    assert len(variants) == 38
    assert {variant.value for variant in variants} == {item["value"] for item in native}
    assert [case[0] for case in INVALID_CASES] == [item["value"] for item in native]
    first_occurrences = sorted(
        Rule,
        key=lambda rule: next(
            i
            for i, item in enumerate(native)
            if item["value"] == rule.variants[0].value
        ),
    )
    assert list(Rule) == first_occurrences
    for rule in Rule:
        assert Rule(rule.value) is rule
        assert rule.name == rule.value.replace("-", "_").upper()
        assert rule.description
        expected = [
            item
            for item in native
            if item["value"] in {variant.value for variant in rule.variants}
        ]
        for variant, item in zip(rule.variants, expected, strict=True):
            assert variant.value == item["value"]
            assert variant.description == item["description"]
            assert variant.arcgis == item["arcgis"]
            assert variant.qgis == item["qgis"]
            assert [kind.value if kind else None for kind in variant.inputs] == item[
                "inputs"
            ]
    assert list(GeometryKind) == [
        GeometryKind.POINT,
        GeometryKind.LINE,
        GeometryKind.POLYGON,
        GeometryKind.COLLECTION,
        GeometryKind.ANY,
    ]
    with pytest.raises(ValueError):
        Rule("LINE_MUST_NOT_HAVE_DANGLES")


@pytest.mark.parametrize(
    "canonical,rule,data,other", INVALID_CASES, ids=[case[0] for case in INVALID_CASES]
)
def test_every_rule_reports_typed_violations(
    canonical: str,
    rule: Rule,
    data: Sequence[BaseGeometry],
    other: Sequence[BaseGeometry] | None,
) -> None:
    """All 38 rules run on meaningful violating inputs and serialize cleanly."""
    topology = Topology()
    primary = topology.add_layer(data, fids=range(-10, -10 + len(data)))
    target = None if other is None else topology.add_layer(other)
    spec = topology.add_rule(rule, primary, other=target)
    assert spec.resolved_rule.value == canonical
    report = topology.validate()
    assert not report.is_valid
    assert report.error_count > 0
    assert report.rules[0].rule is rule
    assert report.rules[0].spec is spec
    assert report.rules[0].resolved_rule is spec.resolved_rule
    assert report.rules[0].layer == primary
    assert report.rules[0].other == target
    assert sum(report.rules[0].counts.values()) == report.error_count
    for violation in report.violations:
        assert violation.rule is rule
        assert violation.resolved_rule is spec.resolved_rule
        assert isinstance(violation.error_type, ErrorType)
        assert isinstance(violation.geometry, BaseGeometry)
        assert not violation.geometry.is_empty
        if violation.origin is not None:
            assert violation.origin.layer in (primary, target)
    assert topology.validate().to_dict() == report.to_dict()
    assert json.loads(json.dumps(report.to_dict()))["error_count"] == report.error_count


EMPTY_VARIANTS = [(rule, variant) for rule in Rule for variant in rule.variants]


@pytest.mark.parametrize(
    "rule,variant", EMPTY_VARIANTS, ids=[variant.value for _, variant in EMPTY_VARIANTS]
)
def test_empty_layers_are_valid_for_every_rule(
    rule: Rule, variant: RuleVariant
) -> None:
    topology = Topology()
    first_kind, second_kind = variant.inputs
    primary = topology.add_layer(
        [], kind=GeometryKind.POINT if first_kind is GeometryKind.ANY else first_kind
    )
    other = (
        None
        if second_kind is None
        else topology.add_layer(
            [],
            kind=GeometryKind.POINT if second_kind is GeometryKind.ANY else second_kind,
        )
    )
    spec = topology.add_rule(rule, primary, other=other)
    assert spec.resolved_rule == variant
    assert topology.validate().is_valid


@pytest.mark.parametrize("rule,data,other", VALID_BINARY_CASES)
def test_binary_rule_valid_examples(
    rule: Rule, data: BaseGeometry, other: BaseGeometry
) -> None:
    topology = Topology()
    primary = topology.add_layer([data])
    target = topology.add_layer([other])
    topology.add_rule(rule, primary, other=target)
    assert topology.validate().is_valid


def test_optional_names_scoped_ids_and_immutable_views() -> None:
    topology = Topology()
    first = topology.add_layer([Point(0, 0)])
    second = topology.add_layer([], name="points")
    third = topology.add_layer([], name="points")
    fourth = topology.add_layer([])
    assert [layer.name for layer in topology.layers] == [
        "layer_0",
        "points",
        "points",
        "layer_3",
    ]
    assert [id.index for id in (first, second, third, fourth)] == [0, 1, 2, 3]
    assert len({first, second, third, fourth}) == 4
    assert repr(first) == "LayerId(0)"
    assert len(topology.layer(first)) == 1
    with pytest.raises(FrozenInstanceError):
        topology.layer(first).name = "changed"  # type: ignore[misc]
    with pytest.raises(AttributeError):
        topology.layers = ()  # type: ignore[misc]
    foreign = Topology().add_layer([])
    assert first != foreign
    with pytest.raises(ValueError, match="belong"):
        topology.layer(foreign)
    with pytest.raises(ValueError, match="belong"):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, first, other=foreign)
    with pytest.raises(TypeError):
        topology.layer(0)  # type: ignore[arg-type]


def test_rule_registration_is_typed_and_atomic() -> None:
    topology = Topology()
    point = topology.add_layer([Point(0, 0)] * 2)
    line = topology.add_layer([EDGE])
    for bad in ("point/must-not-overlap", 0, None, ErrorType.OVERLAP):
        with pytest.raises(TypeError, match="Rule"):
            topology.add_rule(bad, point)  # type: ignore[arg-type]
    with pytest.raises(ValueError):
        topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, point)
    with pytest.raises(ValueError):
        topology.add_rule(Rule.MUST_BE_INSIDE, point)
    with pytest.raises(ValueError):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, point, other=point)
    with pytest.raises(ValueError):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, point, other=line)
    assert topology.validate().rules == ()
    topology.add_rule(Rule.MUST_NOT_OVERLAP, point)
    old = topology.validate()
    topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, line)
    new = topology.validate()
    assert old.error_count == 1 and new.error_count == 3
    assert [result.rule for result in new.rules] == [
        Rule.MUST_NOT_OVERLAP,
        Rule.MUST_NOT_HAVE_DANGLES,
    ]
    assert old.rules[0].errors is old.rules[0].violations
    assert new.errors == new.violations
    with pytest.raises(TypeError):
        new.rules[0].counts[ErrorType.DANGLE] = 9  # type: ignore[index]


@pytest.mark.parametrize(
    "data", [42, "POINT (0 0)", b"x", [object()], {"type": "CircularString"}]
)
def test_unsupported_inputs_do_not_register(data: Any) -> None:
    topology = Topology()
    with pytest.raises(TypeError):
        topology.add_layer(data)
    assert topology.layers == ()
    assert topology.add_layer([]).index == 0


@pytest.mark.parametrize(
    "fids", [[1], [1, 1], [True, 2], [1.5, 2], [-(2**63) - 1, 2], [2**63, 2]]
)
def test_invalid_fids_leave_state_unchanged(fids: Any) -> None:
    topology = Topology()
    with pytest.raises(ValueError):
        topology.add_layer([Point(0, 0), None], fids=fids)
    assert topology.layers == ()


@pytest.mark.parametrize("value", [-1, float("nan"), float("inf")])
def test_invalid_tolerance(value: float) -> None:
    with pytest.raises(ValueError):
        Topology(value)
    with pytest.raises(ValueError):
        merge_lines([], value)


@pytest.mark.parametrize(
    "geometry", [Point(float("inf"), 0), LineString([(0, 0), (float("nan"), 1)])]
)
def test_nonfinite_inputs_leave_registration_unchanged(geometry: BaseGeometry) -> None:
    topology = Topology()
    existing = topology.add_layer([])
    with pytest.raises(ValueError, match="finite"):
        topology.add_layer([geometry])
    assert len(topology.layers) == 1
    new = topology.add_layer([])
    assert new.index == 1 and topology.layer(new).name == "layer_1"
    assert topology.layer(existing).id == existing


def test_missing_empty_and_extra_dimensions_keep_original_fids() -> None:
    topology = Topology()
    id = topology.add_layer([None, Point(), Point(1, 2, 3), float("nan"), Point(1, 2)])
    topology.add_rule(Rule.MUST_NOT_OVERLAP, id)
    layer = topology.layer(id)
    assert layer.fids == (2, 4)
    assert not layer.geometries[0].has_z
    report = topology.validate()
    assert report.error_count == 1
    error = report.violations[0]
    assert error.origin is not None and error.origin.fid == 2
    assert error.destination is not None and error.destination.fid == 4
    assert len(report.warnings) == 4
    assert report.warnings == topology.warnings()


def test_nested_empty_parts_keep_original_provenance() -> None:
    data = shapely.from_wkt(
        "GEOMETRYCOLLECTION (POINT EMPTY, "
        "GEOMETRYCOLLECTION (LINESTRING EMPTY, "
        "MULTILINESTRING (EMPTY, (0 0, 1 0))))"
    )
    topology = Topology()
    id = topology.add_layer([data], fids=[-99])
    topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, id)
    report = topology.validate()
    assert report.error_count == 2
    assert all(
        error.origin is not None and error.origin.fid == -99 and error.origin.part == 3
        for error in report.violations
    )
    assert any("3 empty parts" in warning for warning in report.warnings)
    points = Topology()
    id = points.add_layer([shapely.from_wkt("MULTIPOINT (EMPTY, (1 2), (1 2))")])
    points.add_rule(Rule.MUST_NOT_OVERLAP, id)
    # Parts within one feature are excluded by the point overlap rule.
    assert points.validate().is_valid


def test_core_cleaning_and_aggregated_part_references() -> None:
    topology = Topology()
    id = topology.add_layer([MultiLineString([[(0, 0), (0, 0)], [(1, 0), (2, 0)]])])
    topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, id)
    report = topology.validate()
    assert report.error_count == 2
    assert all(
        error.origin is not None and error.origin.part == 1
        for error in report.violations
    )
    assert any("degenerate" in warning for warning in report.warnings)
    points = Topology()
    id = points.add_layer([MultiPoint([(0, 0), (1, 0)])] * 2)
    points.add_rule(Rule.MUST_NOT_OVERLAP, id)
    report = points.validate()
    assert report.error_count == 1
    assert report.violations[0].origin is not None
    assert report.violations[0].origin.part is None
    assert report.violations[0].geometry.geom_type == "MultiPoint"


def test_all_geometry_families_and_collection_type_checks() -> None:
    topology = Topology()
    multi = topology.add_layer([MultiPolygon([SQUARE])])
    mixed = topology.add_layer([GeometryCollection([Point(0, 0), EDGE, SQUARE])])
    topology.add_rule(Rule.MUST_NOT_BE_MULTIPART, multi)
    topology.add_rule(Rule.MUST_NOT_BE_MULTIPART, mixed)
    assert topology.validate().error_count == 1
    with pytest.raises(ValueError):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, mixed)


def test_geojson_adapters_and_input_mutations() -> None:
    geojson: dict[str, Any] = {
        "type": "FeatureCollection",
        "features": [
            {
                "type": "Feature",
                "id": 99,
                "geometry": {"type": "Point", "coordinates": [0, 0]},
            },
            {"type": "Feature", "geometry": None},
            {"type": "Feature", "geometry": {"type": "Point", "coordinates": [0, 0]}},
        ],
    }
    topology = Topology()
    id = topology.add_layer(geojson)
    geojson["features"][0]["geometry"]["coordinates"][0] = 99
    topology.add_rule(Rule.MUST_NOT_OVERLAP, id)
    assert topology.validate().error_count == 1
    assert topology.layer(id).fids == (0, 2)
    assert topology.layer(id).geometries[0].equals(Point(0, 0))


class GeoObject:
    """Minimal custom GIS adapter."""

    def __init__(self, geometry: BaseGeometry) -> None:
        self.__geo_interface__ = mapping(geometry)


def test_geometry_iterables_and_geo_interface() -> None:
    topology = Topology()
    items: list[Any] = [GeoObject(Point(0, 0)), mapping(Point(0, 0))]
    id = topology.add_layer(iter(items))
    items.clear()
    topology.add_rule(Rule.MUST_NOT_OVERLAP, id)
    assert topology.validate().error_count == 1
    for data in (
        Point(0, 0),
        GeoObject(Point(0, 0)),
        mapping(Point(0, 0)),
        {"type": "Feature", "geometry": mapping(Point(0, 0))},
    ):
        topology = Topology()
        id = topology.add_layer(data)
        assert len(topology.layer(id)) == 1


def test_geopandas_adapters_crs_and_mutation() -> None:
    gpd = pytest.importorskip("geopandas")
    series = gpd.GeoSeries([Point(0, 0), Point(0, 0)], index=[50, 70], crs="EPSG:28992")
    frame = gpd.GeoDataFrame({"shape": series}, geometry="shape")
    for data in (series, frame):
        topology = Topology()
        id = topology.add_layer(data, crs=CRS.from_epsg(28992))
        topology.add_rule(Rule.MUST_NOT_OVERLAP, id)
        assert topology.layer(id).fids == (0, 1)
        assert topology.validate().to_geodataframe().crs == CRS.from_epsg(28992)
    topology = Topology()
    id = topology.add_layer(frame)
    frame.loc[50, "shape"] = Point(99, 99)
    topology.add_rule(Rule.MUST_NOT_OVERLAP, id)
    assert topology.validate().error_count == 1
    with pytest.raises(ValueError, match="conflicts"):
        topology.add_layer(series, crs="EPSG:4326")
    assert len(topology.layers) == 1


def test_crs_consistency_and_unknown_exports() -> None:
    topology = Topology()
    topology.add_layer([], crs="EPSG:4326")
    with pytest.raises(ValueError, match="incompatible"):
        topology.add_layer([], crs="EPSG:3857")
    with pytest.raises(ValueError, match="invalid CRS"):
        topology.add_layer([], crs="invalid")
    assert len(topology.layers) == 1
    assert topology.validate().crs == CRS.from_epsg(4326)
    topology.add_layer([])
    assert topology.validate().crs is None
    assert any("mixed known and unknown" in warning for warning in topology.warnings())
    if importlib.util.find_spec("geopandas") is None:
        with pytest.raises(ImportError, match="GeoDataFrame export requires"):
            topology.validate().to_geodataframe()
    else:
        assert topology.validate().to_geodataframe().crs is None
    geojson: dict[str, Any] = {
        "type": "FeatureCollection",
        "features": [],
        "crs": {"type": "name", "properties": {"name": "EPSG:4326"}},
    }
    fresh = Topology()
    id = fresh.add_layer(geojson, crs=4326)
    assert fresh.layer(id).crs == CRS.from_epsg(4326)


def test_exports_preserve_large_signed_fids_and_absent_refs() -> None:
    pytest.importorskip("geopandas")
    topology = Topology()
    id = topology.add_layer(
        [Point(0, 0)] * 3, fids=[-(2**63), 2**63 - 1, 42], crs=28992
    )
    topology.add_rule(Rule.MUST_NOT_OVERLAP, id)
    polygon = topology.add_layer([SQUARE], name="areas", crs=28992)
    topology.add_rule(Rule.MUST_NOT_HAVE_GAPS, polygon)
    report = topology.validate()
    assert report.error_count == 4
    frame = report.to_geodataframe()
    assert frame.crs == CRS.from_epsg(28992)
    assert frame["origin_fid"].dtype == "Int64"
    assert frame["origin_fid"].tolist()[:3] == [-(2**63), -(2**63), 2**63 - 1]
    assert frame["destination_fid"].tolist()[:3] == [42, 2**63 - 1, 42]
    assert frame["origin_fid"].isna().tolist() == [False, False, False, True]
    assert frame["rule"].tolist()[-1] == Rule.MUST_NOT_HAVE_GAPS.value
    assert frame["error_type"].tolist()[-1] == ErrorType.OUTER_BOUNDARY.value
    assert report.to_dict()["rules"][0]["counts"] == {"overlap": 3}
    assert report.to_dict()["rules"][1]["errors"][0]["origin"] is None
    empty = Topology()
    empty.add_layer([], crs=28992)
    export = empty.validate().to_geodataframe()
    assert export.empty and export.crs == CRS.from_epsg(28992)
    assert export.geometry.name == "geometry"


def test_tolerance_changes_native_validation() -> None:
    def run(tolerance: float) -> int:
        topology = Topology(tolerance)
        id = topology.add_layer([Point(0, 0), Point(0.0005, 0)])
        topology.add_rule(Rule.MUST_NOT_OVERLAP, id)
        assert topology.validate().tolerance == tolerance
        return topology.validate().error_count

    assert run(0) == 0
    assert run(0.001) == 1


def test_native_line_utilities() -> None:
    data = [LineString([(0, 0), (1, 0)]), LineString([(1, 0), (2, 0)])]
    assert merge_lines(data) == (LineString([(0, 0), (1, 0), (2, 0)]),)
    assert explode_lines(merge_lines(data)) == tuple(data)
    assert data[0] == LineString([(0, 0), (1, 0)])
    assert merge_lines([]) == explode_lines([]) == ()
    assert len(merge_lines([data[0], LineString([(1.0005, 0), (2, 0)])], 0.001)) == 1
    assert len(merge_lines([*data, LineString([(1, 0), (1, 1)])])) == 3
    multi = MultiLineString([[(0, 0), (0, 0)], [(2, 0), (3, 0), (4, 0)]])
    assert len(explode_lines(GeometryCollection([multi]))) == 2
    for utility in (merge_lines, explode_lines):
        with pytest.raises(ValueError):
            utility([Point(0, 0)])
        with pytest.raises(ValueError, match="finite"):
            utility([LineString([(0, 0), (float("inf"), 0)])])


def test_operations_on_same_topology_are_serialized() -> None:
    topology = Topology()
    id = topology.add_layer([Point(0, 0)] * 3)
    topology.add_rule(Rule.MUST_NOT_OVERLAP, id)

    def snapshot(_: int) -> dict[str, Any]:
        return topology.validate().to_dict()

    with ThreadPoolExecutor(max_workers=4) as pool:
        reports = list(pool.map(snapshot, range(12)))
    assert all(report == reports[0] for report in reports)
    assert reports[0]["error_count"] == 3


@pytest.mark.parametrize("tolerance", [0.0, 0.001, 0.01, 1.0])
@pytest.mark.parametrize(
    "scale,offset", [(1.0, 0.0), (1.0, 2**20), (0.5, 0.0), (2.0, 0.0)]
)
def test_boundary_band_requires_passages_on_both_sides(
    tolerance: float, scale: float, offset: float
) -> None:
    """Nearby Chicago boundary runs stay contacts under equivalent source frames."""
    shapes = geometry_fixture("boundary_proximity")
    for line, area in zip(shapes[::2], shapes[1::2], strict=True):
        for reverse in [False, True]:
            matrix = [scale, 0.0, 0.0, scale, offset, -offset / 2]
            road = affine_transform(shapely.reverse(line) if reverse else line, matrix)
            polygon = affine_transform(
                shapely.reverse(area) if reverse else area, matrix
            )
            topology = Topology(tolerance=tolerance * scale)
            source = topology.add_layer([OUTSIDE, road], fids=[38, -91])
            target = topology.add_layer([polygon], fids=[872])
            topology.add_rule(Rule.MUST_NOT_CROSS_BOUNDARIES, source, other=target)
            report = topology.validate()
            expected = road.crosses(polygon) if tolerance == 0.0 else False
            assert bool(report.error_count) == expected
            assert all(v.error_type is ErrorType.CROSSING for v in report.violations)
            assert all(v.geometry.is_valid for v in report.violations)
            assert all(
                v.origin is not None and v.origin.fid == -91 for v in report.violations
            )
            assert all(
                v.destination is not None and v.destination.fid == 872
                for v in report.violations
            )
            assert topology.validate().to_dict() == report.to_dict()
            with topology.iter_validate(1) as stream:
                assert [v.to_dict() for v in stream] == [
                    v.to_dict() for v in report.violations
                ]
                assert (
                    stream.summary is not None
                    and stream.summary.error_count == report.error_count
                )


@pytest.mark.parametrize("tolerance", [0.0, 0.001, 1.0])
def test_boundary_band_retains_crossings_and_closed_seams(tolerance: float) -> None:
    """A band joins opposite passages, while same-side departures stay contacts."""
    for before in [-2.0, 2.0]:
        for after in [-2.0, 2.0]:
            for reverse in [False, True]:
                road = LineString([(before, 2), (0, 2), (0, 5), (after, 5)])
                if reverse:
                    road = shapely.reverse(road)
                topology = Topology(tolerance=tolerance)
                topology.add_rule(
                    Rule.MUST_NOT_CROSS_BOUNDARIES, [road], other=[SQUARE]
                )
                report = topology.validate()
                assert report.error_count == int(before != after)
                for v in report.violations:
                    assert v.error_type is ErrorType.CROSSING
                    assert v.geometry.distance(SQUARE.boundary) == 0
    # One crossing lies across the traversal seam of this closed source path.
    coords = [(0, 2), (0, 5), (2, 5), (2, 7), (-2, 7), (-2, 2)]
    for rotation in range(len(coords)):
        points = coords[rotation:] + coords[:rotation]
        road = LineString([*points, points[0]])
        topology = Topology(tolerance=tolerance)
        topology.add_rule(Rule.MUST_NOT_CROSS_BOUNDARIES, [road], other=[SQUARE])
        report = topology.validate()
        assert report.error_count == 1
        assert isinstance(report.violations[0].geometry, MultiPoint)
        assert len(report.violations[0].geometry.geoms) == 2


@pytest.mark.parametrize(
    "scale,offset", [(1.0, 0.0), (1.0, 2**20), (0.5, 0.0), (2.0, 0.0)]
)
@pytest.mark.parametrize("tolerance", [0.0, 0.001, 1.0])
def test_short_junction_passages_do_not_gain_overlap_from_endpoint_caps(
    scale: float, offset: float, tolerance: float
) -> None:
    """Original Iceland and Argentina junctions lack sustained shared linework."""
    from pathlib import Path

    from shapely.affinity import affine_transform

    path = Path(__file__).parents[1] / "crates/core/tests/fixtures/geometry_cases.json"
    data = json.loads(path.read_text())
    shapes = [
        shapely.from_geojson(json.dumps(g))
        for g in data["junction_support"]["geometries"]
    ]
    for a, b in zip(shapes[::2], shapes[1::2], strict=True):
        for reverse in [False, True]:
            matrix = [scale, 0.0, 0.0, scale, offset, -offset / 2]
            pair = [
                affine_transform(shapely.reverse(g) if reverse else g, matrix)
                for g in [a, b]
            ]
            for rule in [
                Rule.MUST_NOT_OVERLAP,
                Rule.MUST_NOT_INTERSECT,
                Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR,
            ]:
                for binary in [False, True]:
                    topology = Topology(tolerance=tolerance * scale)
                    if reverse:
                        pair = pair[::-1]
                    if binary:
                        source = topology.add_layer(pair[:1], fids=[-91])
                        target = topology.add_layer(pair[1:], fids=[872])
                    else:
                        source = topology.add_layer(pair, fids=[-91, 872])
                        target = None
                    topology.add_rule(rule, source, other=target)
                    report = topology.validate()
                    assert all(
                        v.error_type is not ErrorType.OVERLAP for v in report.violations
                    )
                    assert all(v.geometry.is_valid for v in report.violations)
                    assert topology.validate().to_dict() == report.to_dict()
                    with topology.iter_validate(1) as stream:
                        assert [v.to_dict() for v in stream] == [
                            v.to_dict() for v in report.violations
                        ]
                        assert (
                            stream.summary is not None
                            and stream.summary.error_count == report.error_count
                        )
            topology = Topology(tolerance=tolerance * scale)
            parts = [
                list(part.coords)
                for g in pair
                for part in (g.geoms if isinstance(g, MultiLineString) else [g])
            ]
            topology.add_rule(Rule.MUST_NOT_SELF_OVERLAP, [MultiLineString(parts)])
            assert topology.validate().error_count == 0


def test_compact_bent_and_unequal_segmented_nearby_runs_remain_overlaps() -> None:
    """Source traversal length, not planar diameter or segmentation, supports a run."""
    cases = [
        (
            LineString([(-0.6, 0), (-0.6, 0.6), (0.6, 0.6), (0.6, 0), (0, 0)]),
            LineString([(0, 0), (-0.5, 0), (-0.5, 0.5), (0.5, 0.5), (0.5, 0)]),
        ),
        (
            LineString([(i / 2, 0) for i in range(9)]),
            LineString([(0, 0.25), (4, 0.25)]),
        ),
        (LineString([(0, 0), (1.5, 0)]), LineString([(0, 0.25), (1.5, 0.25)])),
        (
            LineString([(-0.75, -0.05), (0.75, 0.05)]),
            LineString([(-0.75, 0.05), (0.75, -0.05)]),
        ),
    ]
    for a, b in cases:
        for binary in [False, True]:
            topology = Topology(tolerance=1)
            topology.add_rule(
                Rule.MUST_NOT_OVERLAP,
                [a] if binary else [a, b],
                other=[b] if binary else None,
            )
            report = topology.validate()
            assert report.error_count == 1
            assert report.violations[0].error_type is ErrorType.OVERLAP
            assert report.violations[0].geometry.is_valid


def test_boundary_coverage_keeps_short_contributions_from_distinct_targets() -> None:
    """Coverage applies its length threshold after combining target intervals."""
    topology = Topology(tolerance=1)
    topology.add_rule(
        Rule.MUST_BE_COVERED_BY_BOUNDARIES,
        [LineString([(0, 0), (3, 0)])],
        other=[box(i / 2, 0, (i + 1) / 2, 10) for i in range(6)],
    )
    assert topology.validate().error_count == 0
    lines = [
        LineString([(i / 2, 0), ((i + 1) / 2, 0), ((i + 1) / 2, -10)]) for i in range(6)
    ]
    lines += [
        LineString([(0, 0), (0, 10)]),
        LineString([(0, 10), (3, 10)]),
        LineString([(3, 0), (3, 10)]),
    ]
    topology = Topology(tolerance=1)
    topology.add_rule(
        Rule.BOUNDARIES_MUST_BE_COVERED_BY, [box(0, 0, 3, 10)], other=lines
    )
    assert topology.validate().error_count == 0


def test_nearby_closed_run_support_survives_source_seam_rotation() -> None:
    """Unexpanded support joins cyclic source passages before overlap filtering."""
    a = [(0.0, 0.0), (0.75, 0.0), (0.75, 5.0), (-0.75, 5.0), (-0.75, 0.0)]
    b = [(0.0, 0.25), (0.75, 0.25), (0.75, -5.0), (-0.75, -5.0), (-0.75, 0.25)]
    for rotate_a in range(len(a)):
        for rotate_b in range(len(b)):
            aa, bb = a[rotate_a:] + a[:rotate_a], b[rotate_b:] + b[:rotate_b]
            for reverse in [False, True]:
                roads = [LineString([*aa, aa[0]]), LineString([*bb, bb[0]])]
                if reverse:
                    roads = [shapely.reverse(g) for g in roads]
                topology = Topology(tolerance=1)
                topology.add_rule(Rule.MUST_NOT_OVERLAP, roads)
                report = topology.validate()
                assert report.error_count == 1
                assert report.violations[0].error_type is ErrorType.OVERLAP
                assert report.violations[0].geometry.is_valid


def test_uncertain_path_prefixes_preserve_original_interval_evidence() -> None:
    """Finite extreme coordinates and large prefixes cannot erase local overlap."""
    cases = [
        (
            LineString([(0, 0), (9e307, 0), (0, 0), (9e307, 0)]),
            LineString([(0, 1e300), (9e307, 1e300), (0, 1e300), (9e307, 1e300)]),
            1e301,
        ),
        (
            LineString([(1e16, 1e16), (0, 1e16), (0, 0), (2, 0)]),
            LineString([(0, 0.25), (1.5, 0.25)]),
            1.0,
        ),
    ]
    for a, b, tolerance in cases:
        topology = Topology(tolerance=tolerance)
        topology.add_rule(Rule.MUST_NOT_OVERLAP, [a], other=[b])
        report = topology.validate()
        assert report.error_count == 1
        assert report.violations[0].error_type is ErrorType.OVERLAP


@pytest.mark.parametrize("offset", [0.0, 0.25])
@pytest.mark.parametrize(
    "scale,translation", [(1.0, 0.0), (0.5, 0.0), (2.0, 0.0), (1.0, 2**20)]
)
def test_shared_support_combines_short_contributions_across_parts(
    offset: float,
    scale: float,
    translation: float,
) -> None:
    """Exact and nearby multipart passages contribute one sustained shared run."""
    a = LineString([(0, 0), (3, 0)])
    parts = [
        [(i / 2, offset), ((i + 1) / 2, offset), ((i + 1) / 2, -10)] for i in range(6)
    ]
    b = MultiLineString(parts)
    from shapely.affinity import affine_transform

    for rule, source, other in [
        (Rule.MUST_NOT_OVERLAP, [a, b], None),
        (Rule.MUST_NOT_OVERLAP, [a], [b]),
        (Rule.MUST_NOT_SELF_OVERLAP, [MultiLineString([list(a.coords), *parts])], None),
    ]:
        matrix = [scale, 0.0, 0.0, scale, translation, -translation / 2]
        topology = Topology(tolerance=scale)
        layer = topology.add_layer(
            [affine_transform(g, matrix) for g in source],
            fids=[-91 + i * 963 for i in range(len(source))],
        )
        target = (
            None
            if other is None
            else topology.add_layer(
                [affine_transform(g, matrix) for g in other], fids=[872]
            )
        )
        topology.add_rule(rule, layer, other=target)
        report = topology.validate()
        assert report.error_count == 1
        assert report.violations[0].error_type in {
            ErrorType.OVERLAP,
            ErrorType.SELF_OVERLAP,
        }
        assert report.violations[0].geometry.is_valid
        if rule is Rule.MUST_NOT_SELF_OVERLAP:
            reference = affine_transform(a, matrix)
            assert (
                report.violations[0].geometry.intersection(reference).length
                >= 3 * scale
            )
        with topology.iter_validate(1) as stream:
            assert [v.to_dict() for v in stream] == [
                v.to_dict() for v in report.violations
            ]


@pytest.mark.parametrize("reverse", [False, True])
def test_nearby_third_passage_keeps_connected_self_support(reverse: bool) -> None:
    """Conflicting short correspondences keep their own rejection decisions."""
    parts = [[(0.0, 0.0), (3.0, 0.0)]] + [
        [(i / 2, 0.25), ((i + 1) / 2, 0.25), ((i + 1) / 2, -10.0)] for i in range(6)
    ]
    parts.append([(0.0, 0.35), (0.5, 0.35), (0.5, 10.0)])
    if reverse:
        parts = [part[::-1] for part in parts[::-1]]
    topology = Topology(tolerance=1.0)
    topology.add_rule(Rule.MUST_NOT_SELF_OVERLAP, [MultiLineString(parts)])
    report = topology.validate()
    horizontal = LineString([(0, 0), (3, 0)])
    assert any(
        v.geometry.intersection(horizontal).length >= 3 for v in report.violations
    )
    with topology.iter_validate(1) as stream:
        assert [v.to_dict() for v in stream] == [v.to_dict() for v in report.violations]


@pytest.mark.parametrize("tolerance", [0.0, 0.001, 1.0])
def test_duplicate_junction_parts_add_no_original_longitudinal_support(
    tolerance: float,
) -> None:
    """Duplicate and unrelated parts cannot restore rejected junction geometry."""
    from pathlib import Path

    path = Path(__file__).parents[1] / "crates/core/tests/fixtures/geometry_cases.json"
    data = json.loads(path.read_text())
    shapes = [
        shapely.from_geojson(json.dumps(g))
        for g in data["junction_support"]["geometries"]
    ]
    for a, b in zip(shapes[::2], shapes[1::2], strict=True):
        a_parts = [
            list(p.coords) for p in (a.geoms if isinstance(a, MultiLineString) else [a])
        ]
        b_parts = [
            list(p.coords) for p in (b.geoms if isinstance(b, MultiLineString) else [b])
        ]
        for duplicate_a in [False, True]:
            topology = Topology(tolerance=tolerance)
            source = topology.add_layer(
                [MultiLineString(a_parts * (2 if duplicate_a else 1))], fids=[-831]
            )
            target = topology.add_layer(
                [MultiLineString(b_parts * (1 if duplicate_a else 2))], fids=[791]
            )
            topology.add_rule(Rule.MUST_NOT_OVERLAP, source, other=target)
            assert topology.validate().error_count == 0


@pytest.mark.parametrize("reverse", [False, True])
def test_disconnected_role_cannot_supply_missing_junction_support(
    reverse: bool,
) -> None:
    """Both multipart passage roles need their own connected supporting union."""
    a = [(0.0, 0.0), (1.5, 0.0)]
    b = [(0.75, 0.5), (10.0, 0.5)]
    c = [(0.75, 1.25), (10.0, 1.25)]
    if reverse:
        a, b, c = a[::-1], b[::-1], c[::-1]
    for self_rule in [False, True]:
        topology = Topology(tolerance=1.0)
        if self_rule:
            topology.add_rule(
                Rule.MUST_NOT_SELF_OVERLAP,
                [MultiLineString([c, b, a] if reverse else [a, b, c])],
            )
        else:
            source = topology.add_layer(
                [MultiLineString([c, a] if reverse else [a, c])]
            )
            target = topology.add_layer([LineString(b)])
            topology.add_rule(Rule.MUST_NOT_OVERLAP, source, other=target)
        report = topology.validate()
        assert report.error_count == 1
        assert report.violations[0].geometry.equals(LineString(b))
        with topology.iter_validate(1) as stream:
            assert [v.to_dict() for v in stream] == [
                v.to_dict() for v in report.violations
            ]
