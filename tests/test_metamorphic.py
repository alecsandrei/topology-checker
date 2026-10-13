"""Independent geometry references and invariance under exact binary transforms."""

from __future__ import annotations

import itertools
import math
import time

import pytest
import shapely
from shapely.affinity import affine_transform
from shapely.geometry import GeometryCollection, LineString, Point, Polygon, box
from shapely.geometry.base import BaseGeometry
from topology_checker import Rule, Topology

from .test_campaign import (
    assert_equivalent,
    case_geometry,
    case_topology,
    geometry,
    run,
)


def transform(g: BaseGeometry, scale: float, offset: float) -> BaseGeometry:
    """Scale by a power of two and translate by exactly representable coordinates."""
    return affine_transform(g, [scale, 0, 0, scale, offset, -offset])


@pytest.mark.parametrize("tolerance", [0.0, 0.01, 1.0])
@pytest.mark.parametrize("scale,offset", [(1.0, 0.0), (2.0, 2**20)])
def test_crossings_survive_connected_boundary_runs(
    tolerance: float, scale: float, offset: float
) -> None:
    """Connected passages well inside and outside cross despite a boundary run."""
    for road, block in [(498, 216), (1401, 329)]:
        line = case_geometry("boundary_run_crossings", f"ROAD_{road}")
        polygon = case_geometry("boundary_run_crossings", f"BLOCK_GROUP_{block}")
        assert line.intersection(polygon.buffer(-0.01)).length > 50
        assert line.difference(polygon.buffer(0.01)).length > 200
        for reverse in [False, True]:
            source = transform(
                shapely.reverse(line) if reverse else line, scale, offset
            )
            target = transform(
                shapely.reverse(polygon) if reverse else polygon, scale, offset
            )
            t = case_topology(
                Rule.MUST_NOT_CROSS_BOUNDARIES,
                [source, transform(LineString([(0, 0), (0, 10)]), scale, offset)],
                [target, transform(box(100, 100, 110, 110), scale, offset)],
                tolerance * scale,
            )
            report = t.validate()
            assert report.error_count >= 1
            if tolerance > 0:
                assert report.error_count == 1
            for v in report.violations:
                assert v.error_type.value == "crossing"
                assert v.origin is not None and v.origin.fid == -91
                assert v.destination is not None and v.destination.fid == 872
                assert v.geometry.distance(target.boundary) < 1e-7 * scale
            assert_equivalent(t)


@pytest.mark.parametrize("tolerance", [0.0, 0.01])
def test_boundary_runs_connect_opposite_sides_and_closed_seams(
    tolerance: float,
) -> None:
    """Opposite sides cross once; a run joining the same side is contact only."""
    polygon = box(0, 0, 10, 10)
    for before, after in itertools.product([-2, 2], repeat=2):
        line = LineString([(before, 2), (0, 2), (0, 5), (after, 5)])
        results: list[BaseGeometry] = []
        for source in [line, shapely.reverse(line)]:
            t = case_topology(
                Rule.MUST_NOT_CROSS_BOUNDARIES, [source], [polygon], tolerance
            )
            report = t.validate()
            assert report.error_count == int(before != after)
            if before != after:
                results.append(report.violations[0].geometry)
            assert_equivalent(t)
        if results:
            assert results[0].equals(results[1])
    vertices = [(-2, 2), (0, 2), (0, 5), (2, 5), (2, 10), (0, 10), (-2, 10)]
    for rotation, reverse in itertools.product(range(len(vertices)), [False, True]):
        ring = vertices[rotation:] + vertices[:rotation]
        if reverse:
            ring.reverse()
        t = case_topology(
            Rule.MUST_NOT_CROSS_BOUNDARIES,
            [LineString([*ring, ring[0]])],
            [polygon],
            tolerance,
        )
        assert t.validate().error_count == 1
        crossings = t.validate().violations[0].geometry
        assert isinstance(crossings, shapely.MultiPoint) and len(crossings.geoms) == 2
        assert shapely.union_all([v.geometry for v in t.validate().violations]).equals(
            shapely.MultiPoint([(0, 2), (0, 10)])
        )
        assert_equivalent(t)


@pytest.mark.parametrize("tolerance", [0.0, 0.001, 1.0])
@pytest.mark.parametrize("binary", [False, True])
@pytest.mark.parametrize(
    "rule",
    [
        Rule.MUST_NOT_OVERLAP,
        Rule.MUST_NOT_INTERSECT,
        Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR,
    ],
)
def test_triangulated_road_network_has_only_endpoint_contacts(
    tolerance: float, binary: bool, rule: Rule
) -> None:
    """A triangulated grid has no shared intervals or interior intersections."""
    lines: list[BaseGeometry] = []
    for x, y in itertools.product(range(15), repeat=2):
        start = (20 * x, 20 * y)
        lines.extend(
            [
                LineString([start, (20 * x + 20, 20 * y)]),
                LineString([start, (20 * x, 20 * y + 20)]),
                LineString([start, (20 * x + 20, 20 * y + 20)]),
            ]
        )
    lines = [shapely.reverse(line) for line in reversed(lines)]
    t = case_topology(
        rule,
        lines[::2] if binary else lines,
        lines[1::2] if binary else None,
        tolerance,
    )
    assert t.validate().is_valid
    assert_equivalent(t)


def test_signed_zero_does_not_reverse_polygon_source_winding() -> None:
    polygon = Polygon([(-0.0, 1), (0.0, 0), (1, 0), (1, 1)])
    expected = box(0, 0, 1, 1)
    (overlap,) = run(Rule.MUST_NOT_OVERLAP, [polygon, expected]).violations
    assert overlap.geometry.is_valid and overlap.geometry.equals(expected)
    (outer,) = run(Rule.MUST_NOT_HAVE_GAPS, [polygon, expected]).violations
    assert isinstance(outer.geometry, LineString)
    assert Polygon(outer.geometry).equals(expected)


@pytest.mark.parametrize("tolerance", [0.0, 2**-60])
def test_line_crossing_uses_exact_interval_midpoints(tolerance: float) -> None:
    polygon = Polygon([(0, 0), (1, 0), (1, 1)])
    line = LineString([(0.5, math.nextafter(0.5, 0)), (0.5, math.nextafter(0.5, 1))])
    assert line.crosses(polygon)
    (crossing,) = run(
        Rule.MUST_NOT_CROSS_BOUNDARIES, [line], [polygon], tolerance
    ).violations
    assert crossing.geometry.equals(Point(0.5, 0.5))


@pytest.mark.parametrize("scale,offset", [(1, 0), (2**-10, 2**8), (2**10, -(2**20))])
def test_polygon_overlay_permutation_rotation_and_reference(
    scale: float, offset: float
) -> None:
    original = [Polygon([(0, 0), (5, 1), (1, 5)]), box(1, 1, 4, 4)]
    for reverse in [False, True]:
        rotated: list[BaseGeometry] = []
        for polygon in original:
            ring = list(polygon.exterior.coords)[:-1]
            ring = ring[1:] + ring[:1]
            if reverse:
                ring.reverse()
            rotated.append(transform(Polygon(ring), scale, offset))
        for order in [(0, 1), (1, 0)]:
            report = run(
                Rule.MUST_NOT_OVERLAP,
                [rotated[i] for i in order]
                + [transform(box(100, 100, 101, 101), scale, offset)],
                fids=[-91, 872, 5],
            )
            (violation,) = report.violations
            expected = rotated[0].intersection(rotated[1])
            assert violation.geometry.is_valid
            assert (
                violation.geometry.symmetric_difference(expected).area
                <= expected.area * 1e-12
            )
            assert violation.origin is not None and violation.destination is not None
            assert {violation.origin.fid, violation.destination.fid} == {-91, 872}
            union = shapely.union_all(rotated)
            gaps = run(Rule.MUST_NOT_HAVE_GAPS, rotated).violations
            outers = [
                v.geometry for v in gaps if v.error_type.value == "outer-boundary"
            ]
            assert len(outers) == 1
            assert isinstance(outers[0], LineString) and outers[0].is_ring
            assert (
                Polygon(outers[0]).symmetric_difference(union).area
                <= union.area * 1e-12
            )


@pytest.mark.parametrize("scale,offset", [(1, 0), (2**-8, 2**4), (2**8, -(2**16))])
def test_line_contacts_feature_order_fids_direction_and_unrelated_input(
    scale: float, offset: float
) -> None:
    lines = [geometry(16, name) for name in ["A", "B"]]
    for order in itertools.permutations(range(2)):
        for reverse in [False, True]:
            transformed = [transform(g, scale, offset) for g in lines]
            if reverse:
                transformed = [shapely.reverse(g) for g in transformed]
            selected = [transformed[i] for i in order]
            report = run(
                Rule.MUST_NOT_HAVE_DANGLES,
                selected
                + [transform(LineString([(100, 100), (101, 101)]), scale, offset)],
                tolerance=0.01 * scale,
                fids=[700, -2, 73],
            )
            # Three source dangles plus both unrelated endpoints.
            assert report.error_count == 5
            assert all(v.geometry.is_valid for v in report.violations)


@pytest.mark.parametrize(
    "rule",
    [
        Rule.MUST_NOT_OVERLAP,
        Rule.MUST_NOT_INTERSECT,
        Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR,
    ],
)
def test_unary_binary_equivalence(rule: Rule) -> None:
    lines = [LineString([(0, 0), (4, 0)]), LineString([(1, 0), (3, 0), (3, 2)])]
    unary = run(rule, lines)
    binary = run(rule, lines[:1], lines[1:])
    assert unary.error_count == binary.error_count
    assert [v.error_type for v in unary.violations] == [
        v.error_type for v in binary.violations
    ]
    assert all(
        a.geometry.equals(b.geometry)
        for a, b in zip(unary.violations, binary.violations, strict=True)
    )


def test_original_part_diagnostics_after_empty_cleanup() -> None:
    spike = geometry(20, "SPIKE_CCW")
    invalid = GeometryCollection([Point(), spike])
    with pytest.raises(ValueError, match=r"FID 91, part 1, ring Some\(0\)"):
        Topology().add_layer([invalid], fids=[91], name="original")
    topology = Topology(tolerance=1)
    layer = topology.add_layer(
        [GeometryCollection([Point(), LineString([(0, 0), (0.1, 0)])])], fids=[91]
    )
    topology.add_rule(Rule.MUST_NOT_BE_MULTIPART, layer)
    assert len(topology.warnings()) == 2
    assert "FID 91, part 1: line collapsed" in topology.warnings()[1]
    assert topology.validate().warnings == topology.warnings()
    with topology.iter_validate() as stream:
        tuple(stream)
    assert stream.summary is not None and stream.summary.warnings == topology.warnings()


def test_cancellation_during_construction_is_prompt() -> None:
    topology = Topology()
    topology.add_rule(
        Rule.MUST_NOT_HAVE_DANGLES,
        [LineString([(i * 3, 0), (i * 3 + 1, 1)]) for i in range(50000)],
    )
    stream = topology.iter_validate(1)
    time.sleep(0.01)
    started = time.monotonic()
    stream.close()
    assert time.monotonic() - started < 3
    assert stream.summary is None
    # A cancelled construction must leave shared caches reusable.
    assert topology.validate().error_count == 100000


def test_indexed_polygon_intersection_preserves_enclosed_holes() -> None:
    large = Polygon(
        [(0, 0), (10, 0), (10, 10), (0, 10)], [[(4, 4), (4, 6), (6, 6), (6, 4)]]
    )
    for small in [
        box(1, 1, 2, 2),
        box(3, 3, 7, 7),
        box(4.2, 4.2, 5.8, 5.8),
        box(11, 11, 12, 12),
    ]:
        report = run(Rule.MUST_NOT_OVERLAP, [large], [small])
        expected = large.intersection(small)
        assert report.error_count == int(expected.area > 0)
        if not expected.is_empty:
            assert report.violations[0].geometry.is_valid
            assert report.violations[0].geometry.equals(expected)


def test_original_ring_diagnostic_after_degenerate_hole_cleanup() -> None:
    invalid = shapely.from_wkt(
        "POLYGON ((0 0,10 0,10 10,0 10,0 0),(1 1,1 1,1 1,1 1),(2 2,4 4,2 4,4 2,2 2))"
    )
    with pytest.raises(ValueError, match=r"FID 812, part 0, ring Some\(2\)"):
        Topology().add_layer([invalid], fids=[812])


def test_duplicate_multipart_pair_aggregates_without_part_product_storage() -> None:
    topology = Topology()
    points = shapely.MultiPoint([Point(0, 0)] * 1000)
    topology.add_rule(Rule.MUST_NOT_OVERLAP, [points, points])
    with topology.iter_validate(1) as stream:
        result = tuple(stream)
    assert len(result) == 1 and result[0].geometry.equals(Point(0, 0))
    assert result[0].origin is not None and result[0].origin.part is None
    assert result[0].destination is not None and result[0].destination.part is None


@pytest.mark.parametrize(
    "rule",
    [
        Rule.MUST_NOT_OVERLAP,
        Rule.MUST_NOT_INTERSECT,
        Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR,
    ],
)
@pytest.mark.parametrize("binary", [False, True])
@pytest.mark.parametrize("tolerance", [0.0, 0.125])
@pytest.mark.parametrize(
    "scale,offset", [(1.0, 0.0), (2.0**-8, 256.0), (256.0, -65536.0)]
)
def test_nearby_bent_intervals_with_unequal_segmentation(
    rule: Rule, binary: bool, tolerance: float, scale: float, offset: float
) -> None:
    """Equal and unequal segmentation describe the same longitudinal interval."""
    delta = 0.0 if tolerance == 0 else 2.0**-10
    a = LineString([(0, 0), (5, 1), (10, 0)])
    b = LineString(
        [
            (0, delta),
            (2.5, 0.5 + delta),
            (5, 1 + delta),
            (7.5, 0.5 + delta),
            (10, delta),
        ]
    )
    for reverse, order in itertools.product([False, True], [(0, 1), (1, 0)]):
        geoms = [
            affine_transform(g, [scale, 0, 0, scale, offset, -offset]) for g in [a, b]
        ]
        if reverse:
            geoms = [shapely.reverse(g) for g in geoms]
        x, y = [geoms[i] for i in order]
        far = affine_transform(
            LineString([(100, 100), (110, 100)]), [scale, 0, 0, scale, offset, -offset]
        )
        topology = case_topology(
            rule,
            [x, far] if binary else [x, y, far],
            [y] if binary else None,
            tolerance * scale,
        )
        (violation,) = topology.validate().violations
        assert violation.error_type.value == "overlap"
        assert violation.geometry.length == pytest.approx(a.length * scale, rel=1e-10)
        assert violation.geometry.hausdorff_distance(x) <= max(delta * scale * 2, 1e-9)
        assert violation.origin is not None and violation.origin.fid == -91
        assert violation.destination is not None and violation.destination.fid == (
            872 if binary else -90
        )
        assert_equivalent(topology)


@pytest.mark.parametrize("tolerance", [0.0, 0.125, 1.0])
def test_nearby_boundary_intervals_in_both_coverage_directions(
    tolerance: float,
) -> None:
    """Nearby top edges are contact; the remaining three sides total 20 units."""
    delta = 0.0 if tolerance == 0 else 2.0**-10
    line = LineString([(0, delta), (5, 1 + delta), (10, delta)])
    polygon = Polygon([(0, 0), (5, 1), (10, 0), (10, -5), (0, -5)])
    for reverse in [False, True]:
        target = shapely.reverse(polygon) if reverse else polygon
        for rule in [
            Rule.MUST_NOT_OVERLAP_BOUNDARIES,
            Rule.MUST_NOT_OVERLAP,
            Rule.MUST_NOT_INTERSECT,
        ]:
            topology = case_topology(rule, [line], [target], tolerance)
            (v,) = topology.validate().violations
            assert v.geometry.length == pytest.approx(line.length)
            assert v.origin is not None and v.origin.fid == -91
            assert v.destination is not None and v.destination.fid == 872
            assert_equivalent(topology)
        assert (
            case_topology(
                Rule.MUST_BE_COVERED_BY_BOUNDARIES, [line], [target], tolerance
            )
            .validate()
            .is_valid
        )
        assert (
            case_topology(Rule.MUST_NOT_CROSS_BOUNDARIES, [line], [target], tolerance)
            .validate()
            .is_valid
        )
        topology = case_topology(
            Rule.BOUNDARIES_MUST_BE_COVERED_BY, [target], [line], tolerance
        )
        assert sum(
            v.geometry.length for v in topology.validate().violations
        ) == pytest.approx(20.0)
        assert_equivalent(topology)


def test_chicago_near_boundary_interval_is_contact() -> None:
    """The tiny interior excursion remains within 1 mm of a shared boundary run."""
    a, b = [case_geometry("chicago_boundary", key) for key in ["A", "B"]]
    assert a.crosses(b)
    assert a.difference(b).length > 0
    for reverse in [False, True]:
        x = shapely.reverse(a) if reverse else a
        assert (
            case_topology(Rule.MUST_NOT_CROSS_BOUNDARIES, [x], [b], 0.001)
            .validate()
            .is_valid
        )
        assert (
            case_topology(Rule.MUST_NOT_CROSS_BOUNDARIES, [x], [b], 0.0)
            .validate()
            .error_count
            == 1
        )


@pytest.mark.parametrize("tol", [0.0, 0.001])
def test_tfl_crossing_survives_clustering(tol: float) -> None:
    """Original nonadjacent passages cross more than tolerance from open endpoints."""
    line = case_geometry("tfl_self_crossing", "A")
    assert isinstance(line, LineString) and not line.is_simple
    crossing = LineString(list(line.coords)[:2]).intersection(
        LineString(list(line.coords)[2:4])
    )
    assert crossing.geom_type == "Point"
    assert crossing.distance(shapely.Point(line.coords[0])) > 0.001
    for reverse in [False, True]:
        x = shapely.reverse(line) if reverse else line
        topology = case_topology(
            Rule.MUST_NOT_SELF_INTERSECT, [x, LineString([(0, 0), (1, 0)])], None, tol
        )
        crossings = [
            v
            for v in topology.validate().violations
            if v.error_type.value == "self-crossing"
        ]
        assert len(crossings) == (2 if tol == 0 else 1)
        assert any(v.geometry.distance(crossing) < 1e-8 for v in crossings)
        assert any(
            p.distance(crossing) < 1e-8
            for v in crossings
            for p in shapely.get_parts(v.geometry)
        )
        assert crossings[0].origin is not None and crossings[0].origin.fid == -91
        assert_equivalent(topology)


def test_near_duplicate_roads_and_projected_endpoint_cuts() -> None:
    """Nanometre road differences and projected endpoint cuts retain full runs."""
    a, b = [case_geometry("tiger_near_duplicate", key) for key in ["ROAD_A", "ROAD_B"]]
    for rule in [
        Rule.MUST_NOT_OVERLAP,
        Rule.MUST_NOT_INTERSECT,
        Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR,
    ]:
        topology = case_topology(rule, [a], [b], 1.0)
        (v,) = topology.validate().violations
        assert v.error_type.value == "overlap" and v.geometry.length == pytest.approx(
            51.802, abs=0.001
        )
        assert_equivalent(topology)
    a = LineString([(0, 0), (5, 1), (10, 0)])
    b = affine_transform(a, [1, 0, 0, 1, 2**-10, 0])
    (v,) = case_topology(Rule.MUST_NOT_OVERLAP, [a], [b], 1.0).validate().violations
    assert v.geometry.length == pytest.approx(a.length)


@pytest.mark.parametrize("tol", [0.0, 0.001, 0.1])
def test_long_routes_are_clipped_against_segment_local_polygon_union(
    tol: float,
) -> None:
    """Independent union clipping preserves outside pieces and original vertices."""
    lines = [
        LineString([(i / 8, y + math.sin(i / 8)) for i in range(801)]) for y in [0, 10]
    ]
    polygons = [box(x, -5, x + 3, 20) for x in range(0, 100, 10)]
    for scale, offset in [(1.0, 0.0), (2.0, 2.0**20)]:
        for reverse in [False, True]:
            inputs = [
                transform(shapely.reverse(g) if reverse else g, scale, offset)
                for g in lines
            ]
            targets = [
                transform(shapely.reverse(g) if reverse else g, scale, offset)
                for g in polygons
            ]
            if reverse:
                targets.reverse()
            targets.append(box(-(2.0**30), -(2.0**30), -(2.0**30) + 1, -(2.0**30) + 1))
            expected = [g.difference(shapely.union_all(targets)) for g in inputs]
            t = Topology(tolerance=tol * scale)
            a = t.add_layer(inputs, fids=[-91, 872])
            b = t.add_layer(targets, fids=list(range(900, 900 + len(targets))))
            t.add_rule(Rule.MUST_BE_INSIDE, a, other=b)
            report = t.validate()
            assert report.error_count == 2
            for v in report.violations:
                assert v.origin is not None and v.destination is None
                reference = expected[0 if v.origin.fid == -91 else 1]
                assert v.geometry.is_valid
                assert v.geometry.hausdorff_distance(reference) < 1e-8 * scale
                assert v.geometry.length == pytest.approx(reference.length, rel=1e-10)
            assert_equivalent(t)


@pytest.mark.parametrize("tolerance", [0.0, 0.001, 1.0])
@pytest.mark.parametrize("scale,offset", [(1.0, 0.0), (2**-3, 2**20)])
def test_original_junctions_and_short_bends_have_no_shared_linework(
    tolerance: float, scale: float, offset: float
) -> None:
    """Independent point intersections and a simple shoreline forbid overlap."""
    for prefix, ids in [("WATERCOURSE", (3058, 3082)), ("ROAD", (819, 1066))]:
        a, b = [case_geometry("junction_contacts", f"{prefix}_{fid}") for fid in ids]
        assert a.intersection(b).geom_type == "Point"
        for reverse, binary in itertools.product([False, True], repeat=2):
            lines = [transform(g, scale, offset) for g in [a, b]]
            if reverse:
                lines = [shapely.reverse(g) for g in reversed(lines)]
            for rule in [
                Rule.MUST_NOT_OVERLAP,
                Rule.MUST_NOT_INTERSECT,
                Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR,
            ]:
                t = case_topology(
                    rule,
                    lines[:1] if binary else lines,
                    lines[1:] if binary else None,
                    tolerance * scale,
                )
                assert all(
                    v.error_type.value != "overlap" for v in t.validate().violations
                )
                if rule != Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR:
                    assert t.validate().is_valid
                assert_equivalent(t)
    shore = case_geometry("junction_contacts", "SHORE_20284")
    assert shore.is_simple
    coordinates = list(shapely.get_parts(shore))[0].coords
    vertices = list(coordinates)[:-1]
    for rotation, reverse in itertools.product([0, 7, 16], [False, True]):
        ring = vertices[rotation:] + vertices[:rotation]
        if reverse:
            ring.reverse()
        line = transform(LineString([*ring, ring[0]]), scale, offset)
        for rule in [Rule.MUST_NOT_SELF_OVERLAP, Rule.MUST_NOT_SELF_INTERSECT]:
            t = case_topology(
                rule,
                [line, transform(LineString([(0, 0), (0, 100)]), scale, offset)],
                None,
                tolerance * scale,
            )
            assert t.validate().is_valid
            assert_equivalent(t)
