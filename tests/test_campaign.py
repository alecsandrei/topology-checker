"""Affirmative regressions from the 2026-10-11 external campaign."""

from __future__ import annotations

import json
import random
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import cast

import pytest
import shapely
from shapely.geometry import LineString, Point, Polygon, box
from shapely.geometry.base import BaseGeometry
from topology_checker import Report, Rule, Topology

FIXTURES = Path(__file__).parent / "fixtures" / "campaign"
DATA: dict[str, dict[str, str | list[str]]] = json.loads(
    (FIXTURES / "geometries.json").read_text()
)


def geometry(bug: int, name: str) -> BaseGeometry:
    """Read an original minimized geometry without running bug-present assertions."""
    return shapely.from_wkt(cast(str, DATA[f"BUG-{bug:03}"][name]))


def run(
    rule: Rule,
    inputs: Sequence[BaseGeometry],
    other: Sequence[BaseGeometry] | None = None,
    tolerance: float = 0.0,
    fids: Sequence[int] | None = None,
) -> Report:
    """Validate rebuilt native code using explicit immutable layers."""
    topology = Topology(tolerance=tolerance)
    layer = topology.add_layer(inputs, name="source", fids=fids)
    target = None if other is None else topology.add_layer(other, name="target")
    topology.add_rule(rule, layer, other=target)
    return topology.validate()


@pytest.mark.parametrize("tol", [0.0, 0.001])
def test_bug_001_boundary_runs_do_not_cross(tol: float) -> None:
    assert run(
        Rule.MUST_NOT_CROSS_BOUNDARIES,
        [geometry(1, "LINE")],
        [geometry(1, "POLYGON")],
        tol,
    ).is_valid


@pytest.mark.parametrize("rule", [Rule.MUST_NOT_OVERLAP, Rule.MUST_NOT_INTERSECT])
@pytest.mark.parametrize("tol", [0.0, 0.001, 0.1])
def test_bug_002_003_shared_edge_is_line_geometry(rule: Rule, tol: float) -> None:
    edge, triangle = geometry(2, "EDGE"), geometry(2, "TRIANGLE")
    (v,) = run(rule, [edge], [triangle], tol).violations
    assert v.geometry.equals(edge)
    assert v.geometry.is_valid
    assert v.origin is not None and v.origin.fid == 0 and v.origin.part == 0
    assert v.destination is not None and v.destination.fid == 0


def test_bug_004_boundary_coverage_survives_distant_features() -> None:
    for polygons in [
        [geometry(4, "TRIANGLE")],
        [geometry(4, "TRIANGLE"), geometry(4, "FAR_BOX")],
    ]:
        assert run(Rule.MUST_BE_INSIDE, [geometry(4, "LINE")], polygons, 0.001).is_valid
    assert run(
        Rule.MUST_BE_INSIDE, [geometry(4, "BASE")], [geometry(4, "THIN")], 0.001
    ).is_valid


@pytest.mark.parametrize(
    "bug,name",
    [
        (5, "BOWTIE"),
        (7, "NESTED"),
        (7, "OVERLAPPING"),
        (20, "SPIKE_CCW"),
        (20, "SPIKE_CW"),
    ],
)
def test_bug_005_007_020_invalid_polygons_rejected_atomically(
    bug: int, name: str
) -> None:
    topology = Topology()
    with pytest.raises(ValueError, match=r"layer .*FID 77.*part 0.*(ring|multipart)"):
        topology.add_layer([geometry(bug, name)], name="invalid", fids=[77])
    assert topology.layers == ()
    with pytest.raises(ValueError, match="invalid polygon"):
        topology.add_rule(Rule.MUST_NOT_HAVE_GAPS, [geometry(bug, name)])
    assert topology.layers == () and topology.validate().rules == ()


def test_bug_006_ring_rotation_has_no_endpoint() -> None:
    line = geometry(6, "LINE")
    polygon = cast(Polygon, geometry(6, "CORNER_FIRST"))
    vertices = list(polygon.exterior.coords)[:-1]
    for rule in [Rule.MUST_NOT_OVERLAP_BOUNDARIES, Rule.MUST_BE_COVERED_BY_BOUNDARIES]:
        results = [
            run(rule, [line], [Polygon(vertices[i:] + vertices[:i])], 1).to_dict()
            for i in range(4)
        ]
        assert all(result == results[0] for result in results)


@pytest.mark.parametrize("tol", [0.0, 1e-9, 0.001])
def test_bug_008_boundary_is_never_properly_inside(tol: float) -> None:
    (v,) = run(
        Rule.MUST_BE_PROPERLY_INSIDE,
        [geometry(8, "EDGE")],
        [geometry(8, "TRIANGLE")],
        tol,
    ).violations
    assert v.error_type.value == "outside" and v.geometry.equals(geometry(8, "EDGE"))


@pytest.mark.parametrize(
    "rule", [Rule.MUST_NOT_SELF_OVERLAP, Rule.MUST_NOT_SELF_INTERSECT]
)
def test_bug_009_self_rules_ignore_other_features(rule: Rule) -> None:
    feature = geometry(9, "FEATURE")
    assert run(rule, [feature], tolerance=1).is_valid
    result = run(
        rule, [feature, geometry(9, "CROSS_A"), geometry(9, "CROSS_B")], tolerance=1
    )
    assert not [
        v for v in result.violations if v.origin is not None and v.origin.fid == 0
    ]


def test_bug_010_valid_overlap_from_valid_multipart_inputs() -> None:
    a, b = geometry(10, "PARCEL"), geometry(10, "IRIS")
    (v,) = run(Rule.MUST_NOT_OVERLAP, [a], [b], 0.001).violations
    assert v.geometry.is_valid, shapely.is_valid_reason(v.geometry)
    assert v.geometry.area == pytest.approx(a.intersection(b).area, abs=2e-5)


def test_bug_011_collapsed_parts_warn_once() -> None:
    topology = Topology(tolerance=1)
    layer = topology.add_layer(
        [LineString([(0, -0.3), (0, 0.3)])], name="tiny", fids=[42]
    )
    topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, layer)
    warnings = topology.warnings()
    assert (
        len(warnings) == 1
        and "FID 42" in warnings[0]
        and "part 0" in warnings[0]
        and "collapsed" in warnings[0]
    )
    assert topology.validate().warnings == warnings
    assert topology.validate().is_valid


def test_bug_012_open_endpoint_crossing_exemption() -> None:
    for end in [99.5, 100.1, 101.0]:
        assert run(
            Rule.MUST_NOT_CROSS_BOUNDARIES,
            [LineString([(50, 50), (end, 50)])],
            [box(0, 0, 100, 100)],
            1,
        ).is_valid
    assert (
        run(
            Rule.MUST_NOT_CROSS_BOUNDARIES,
            [LineString([(50, 50), (102, 50)])],
            [box(0, 0, 100, 100)],
            1,
        ).error_count
        == 1
    )


def test_closed_line_boundary_crossings_have_no_endpoint_exemption() -> None:
    vertices = [(100.1, 50), (99.5, 50), (99.5, 52), (100.1, 52)]
    expected = shapely.MultiPoint([(100, 50), (100, 52)])
    for start in range(len(vertices)):
        ring = vertices[start:] + vertices[:start]
        (violation,) = run(
            Rule.MUST_NOT_CROSS_BOUNDARIES,
            [LineString([*ring, ring[0]])],
            [box(0, 0, 100, 100)],
            1,
        ).violations
        assert violation.geometry.equals(expected)


def test_bug_013_crs_axis_order_equivalence() -> None:
    topology = Topology()
    a = topology.add_layer([Point(0, 0)], crs="EPSG:4326")
    b = topology.add_layer([Point(0, 0)], crs="OGC:CRS84")
    topology.add_rule(Rule.MUST_NOT_OVERLAP, a, other=b)
    assert topology.validate().error_count == 1
    with pytest.raises(ValueError, match="incompatible"):
        topology.add_layer([Point(0, 0)], crs="EPSG:3857")


def test_bug_014_gap_components_preserve_local_topology() -> None:
    a, b = geometry(14, "A"), geometry(14, "B")
    alone = run(Rule.MUST_NOT_HAVE_GAPS, [a, b], tolerance=1)
    far = run(Rule.MUST_NOT_HAVE_GAPS, [a, b, geometry(14, "FAR")], tolerance=1)
    assert alone.error_count == 1 and far.error_count == 2
    assert Polygon(alone.violations[0].geometry.coords).area == pytest.approx(
        a.union(b).area
    )


@pytest.mark.parametrize("d", [0.0009, 0.0010001, 0.0011, 0.002])
def test_bug_015_junction_connectors_are_not_overlap(d: float) -> None:
    lines = [geometry(15, "A"), geometry(15, "B"), LineString([(0, -d), (-5.85, 8.1)])]
    assert run(Rule.MUST_NOT_OVERLAP, lines, tolerance=0.001).is_valid
    for rule in [Rule.MUST_NOT_INTERSECT, Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR]:
        assert all(
            v.geometry.length == 0 for v in run(rule, lines, tolerance=0.001).violations
        )


def test_bug_016_feature_order_and_fid_independence() -> None:
    a, b = geometry(16, "A"), geometry(16, "B")
    reports = [
        run(Rule.MUST_NOT_HAVE_DANGLES, [a, b], tolerance=0.01),
        run(Rule.MUST_NOT_HAVE_DANGLES, [b, a], tolerance=0.01, fids=[91, -4]),
    ]
    assert [r.error_count for r in reports] == [3, 3]
    assert shapely.union_all([v.geometry for v in reports[0].violations]).equals(
        shapely.union_all([v.geometry for v in reports[1].violations])
    )


@pytest.mark.parametrize(
    "rule",
    [
        Rule.MUST_NOT_SELF_OVERLAP,
        Rule.MUST_NOT_SELF_INTERSECT,
        Rule.MUST_NOT_HAVE_PSEUDO_NODES,
    ],
)
def test_bug_017_line_reversal(rule: Rule) -> None:
    a, b = geometry(17, "A"), geometry(17, "B")
    forward = run(rule, [a, b], tolerance=1)
    backward = run(rule, [shapely.reverse(a), b], tolerance=1)
    assert forward.to_dict() == backward.to_dict()


def test_bug_018_release_gaps_finishes_correctly() -> None:
    source = """import json, sys, shapely
from topology_checker import Rule, Topology
parts = [shapely.from_wkt(w) for w in json.load(open(sys.argv[1]))]
t = Topology(tolerance=1)
t.add_rule(Rule.MUST_NOT_HAVE_GAPS, parts)
r = t.validate()
expected = shapely.union_all(parts)
outer = [shapely.Polygon(v.geometry.coords) for v in r.violations
         if v.error_type.value == "outer-boundary"]
pieces = list(shapely.get_parts(expected))
assert len(outer) == len(pieces)
assert all(p.is_valid for p in outer)
expected_outer = shapely.union_all([shapely.Polygon(p.exterior) for p in pieces])
difference = shapely.union_all(outer).symmetric_difference(expected_outer)
assert difference.area <= expected_outer.area * 1e-12
holes = [shapely.Polygon(ring) for p in pieces for ring in p.interiors]
holes = [p for p in holes if p.area > 0 and 2 * p.area / p.length >= 1]
gaps = [shapely.Polygon(v.geometry.coords) for v in r.violations
        if v.error_type.value == "gap"]
assert len(gaps) == len(holes)
assert all(p.is_valid for p in gaps)
assert all(v.origin is None and v.destination is None for v in r.violations)
"""
    result = subprocess.run(
        [sys.executable, "-c", source, str(FIXTURES / "BUG-018_parts.json")],
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode == 0, result.stderr


def test_bug_019_zero_tolerance_positive_area_decisions() -> None:
    data: dict[str, str] = json.loads((FIXTURES / "BUG-019.json").read_text())
    tract, zip_code, ward3, ward4 = [
        shapely.from_wkt(data[k]) for k in ["tract", "zip", "ward3", "ward4"]
    ]
    assert run(Rule.MUST_NOT_OVERLAP, [tract], [zip_code]).is_valid
    (v,) = run(Rule.MUST_NOT_OVERLAP, [ward3, ward4]).violations
    assert v.geometry.is_valid
    assert v.geometry.area == pytest.approx(ward3.intersection(ward4).area, rel=0.01)


def test_bug_021_collapsed_lines_cannot_grow_spikes() -> None:
    lines = [shapely.from_wkt(w) for w in cast(list[str], DATA["BUG-021"]["LINES"])]
    for rule in [
        Rule.MUST_NOT_SELF_OVERLAP,
        Rule.MUST_NOT_SELF_INTERSECT,
        Rule.MUST_NOT_OVERLAP,
    ]:
        report = run(rule, lines, tolerance=1)
        assert not [
            v
            for v in report.violations
            if v.origin is not None and v.origin.fid in [1, 2]
        ]
    report = run(Rule.MUST_BE_COVERED_BY_BOUNDARIES, lines, [box(-50, -50, 50, 50)], 1)
    assert {v.origin.fid for v in report.violations if v.origin is not None} == {0, 3}


def test_boundary_edge_property_2000_triangles() -> None:
    rng = random.Random(20261011)
    for _ in range(2000):
        vertices = [
            (rng.uniform(200000, 300000), rng.uniform(400000, 500000)) for _ in range(3)
        ]
        triangle = Polygon(vertices)
        edge = LineString(vertices[:2])
        for tol in [0.0, 0.001]:
            for rule in [
                Rule.MUST_BE_INSIDE,
                Rule.MUST_NOT_CROSS_BOUNDARIES,
                Rule.MUST_BE_COVERED_BY_BOUNDARIES,
            ]:
                assert run(rule, [edge], [triangle], tol).is_valid
            for rule in [
                Rule.MUST_NOT_OVERLAP,
                Rule.MUST_NOT_INTERSECT,
                Rule.MUST_NOT_OVERLAP_BOUNDARIES,
                Rule.MUST_BE_PROPERLY_INSIDE,
            ]:
                (v,) = run(rule, [edge], [triangle], tol).violations
                assert v.geometry.is_valid and v.geometry.equals(edge)

            (uncovered,) = run(
                Rule.BOUNDARIES_MUST_BE_COVERED_BY, [triangle], [edge], tol
            ).violations
            assert uncovered.geometry.is_valid
            assert uncovered.geometry.equals(triangle.boundary.difference(edge))
            assert uncovered.origin is not None and uncovered.origin.fid == 0
