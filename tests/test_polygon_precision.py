"""Exact polygon decisions, independent reference counts, and failure atomicity."""

from __future__ import annotations

import itertools
import math
from fractions import Fraction

import pytest
import shapely
from shapely.geometry import GeometryCollection, Polygon, box
from topology_checker import PrecisionError, Rule, Topology

from .fraction_reference import point, signed_area, union_boundary

TRIANGLE = [
    (-1782079.763005541, -2764781.752256425),
    (-2115977.5161031107, 2038587.3908392591),
    (-2115977.5161031107, 2038587.390839259),
]
AREA = Fraction(1434079929761945, 36893488147419103232)


def test_triangle_positive_area_decision_for_every_rotation_and_reversal() -> None:
    """The same exact positive overlap must survive cancellation in every ring order."""
    outputs: list[bytes] = []
    for scale in [2.0**-10, 1.0, 2.0**10]:
        for start, reverse in itertools.product(range(3), [False, True]):
            coords = TRIANGLE[start:] + TRIANGLE[:start]
            if reverse:
                coords = list(reversed(coords))
            coords = [(x * scale, y * scale) for x, y in coords]
            ring = [point(x, y) for x, y in [*coords, coords[0]]]
            assert abs(signed_area(ring)) == AREA * Fraction(scale) ** 2
            topology = Topology()
            source = topology.add_layer([Polygon(coords)], fids=[-91])
            target = topology.add_layer([box(-1e12, -1e12, 1e12, 1e12)], fids=[872])
            topology.add_rule(Rule.MUST_NOT_OVERLAP, source, other=target)
            (v,) = topology.validate().violations
            assert v.geometry.is_valid
            assert v.origin is not None and v.origin.fid == -91
            assert v.destination is not None and v.destination.fid == 872
            with topology.iter_validate(1) as stream:
                assert [item.to_dict() for item in stream] == [v.to_dict()]
                assert stream.summary is not None
            if scale == 1:
                outputs.append(v.geometry.wkb)
    assert len(set(outputs)) == 1


def test_multipart_aggregation_precedes_geometry_emission() -> None:
    """Duplicate source pieces union once, before their intersection is rounded."""
    triangle = Polygon(TRIANGLE)
    source = GeometryCollection([triangle, shapely.reverse(triangle)])
    topology = Topology()
    topology.add_rule(
        Rule.MUST_NOT_OVERLAP, [source], other=[box(-1e12, -1e12, 1e12, 1e12)]
    )
    (v,) = topology.validate().violations
    assert isinstance(v.geometry, Polygon) and v.geometry.is_valid
    assert (
        abs(signed_area([point(x, y) for x, y in v.geometry.exterior.coords])) == AREA
    )
    assert v.origin is not None and v.origin.part is None
    assert v.destination is not None and v.destination.part == 0


def precision_failure() -> Topology:
    """Create an exact intersection between two adjacent binary source vertices."""
    x = math.nextafter(1.0, 2.0)
    topology = Topology()
    layer = topology.add_layer(
        [box(1, 0, x, 1), Polygon([(1, -1), (x, -1), (1, 1)])],
        name="narrow",
        fids=[-17, 82],
    )
    topology.add_rule(Rule.MUST_NOT_HAVE_GAPS, layer)
    return topology


def test_precision_error_eager_stream_cache_and_repeated_requests() -> None:
    """Failures carry context, do not poison caches, and have no successful summary."""
    topology = precision_failure()
    for _ in range(2):
        with pytest.raises(
            PrecisionError, match="polygon union emission.*narrow"
        ) as error:
            topology.validate()
        assert isinstance(error.value, RuntimeError)
        with topology.iter_validate(1) as stream:
            with pytest.raises(PrecisionError, match="polygon union emission.*narrow"):
                list(stream)
            assert stream.summary is None
    topology.add_layer([box(100, 100, 101, 101)], name="unrelated")
    with pytest.raises(PrecisionError):
        topology.validate()


@pytest.mark.parametrize("reverse", [False, True])
def test_reference_reconstructs_articulation_holes_from_source_intervals(
    reverse: bool,
) -> None:
    """Two empty lobes that meet at one vertex remain separate bounded regions."""
    shape = Polygon(
        [(0, 0), (4, 0), (4, 4), (0, 4)],
        holes=[[(1, 1), (2, 1), (2, 2), (1, 2)], [(2, 2), (3, 2), (3, 3), (2, 3)]],
    )
    if reverse:
        shape = shapely.reverse(shape)
    reference = union_boundary([shape, box(100, 100, 101, 101)])
    assert len(reference.holes) == 2 and len(reference.shells) == 2
    assert [signed_area(r) for r in reference.holes] == [1, 1]
    topology = Topology()
    topology.add_rule(Rule.MUST_NOT_HAVE_GAPS, [shape, box(100, 100, 101, 101)])
    assert topology.validate().error_count == 4


def test_reference_intersections_and_hidden_intervals() -> None:
    """Boundary classification uses original coverage, including nested inputs."""
    reference = union_boundary(
        [
            box(0, 0, 3, 1),
            box(0, 2, 3, 3),
            box(0, 1, 1, 2),
            box(2, 1, 3, 2),
            box(0.1, 0.1, 0.9, 0.9),
        ]
    )
    assert len(reference.holes) == 1 and len(reference.shells) == 1
    assert signed_area(reference.holes[0]) == 1


def test_bbox_contacts_do_not_clear_positive_overlap_part_provenance() -> None:
    """A noncontributing multipart member cannot erase the part of an overlap."""
    source = GeometryCollection([box(0, 0, 1, 1), box(2, 2, 3, 3)])
    topology = Topology()
    topology.add_rule(Rule.MUST_NOT_OVERLAP, [source], other=[box(0.5, 0.5, 2, 2)])
    (v,) = topology.validate().violations
    assert v.origin is not None and v.origin.part == 0
    assert v.destination is not None and v.destination.part == 0


def test_adjacent_polygon_collection_feature_emits_its_exact_union() -> None:
    """Point-count and containment violations cannot create an invalid multipolygon."""
    source = GeometryCollection([box(0, 0, 1, 1), box(1, 0, 2, 1)])
    t = Topology()
    t.add_rule(Rule.MUST_CONTAIN, [source], other=[])
    (v,) = t.validate().violations
    assert v.geometry.is_valid and v.geometry.equals(box(0, 0, 2, 1))
    with t.iter_validate(1) as stream:
        assert [item.to_dict() for item in stream] == [v.to_dict()]
    contained = Topology()
    contained.add_rule(
        Rule.MUST_NOT_BE_CONTAINED_BY, [source], other=[box(-1, -1, 3, 2)]
    )
    (v,) = contained.validate().violations
    assert v.geometry.is_valid and v.geometry.equals(box(0, 0, 2, 1))


def test_feature_emission_precision_error_keeps_rule_layer_and_fid() -> None:
    """Feature aggregation has the same typed precision failure as gap extraction."""
    x = math.nextafter(1.0, 2.0)
    source = GeometryCollection([box(1, 0, x, 1), Polygon([(1, -1), (x, -1), (1, 1)])])
    t = Topology()
    layer = t.add_layer([source], name="aggregate", fids=[-91])
    t.add_rule(Rule.MUST_CONTAIN, layer, other=[])
    with pytest.raises(
        PrecisionError, match="polygon feature emission.*aggregate.*FID -91"
    ):
        t.validate()
    with t.iter_validate(1) as stream:
        with pytest.raises(PrecisionError, match="polygon feature emission.*FID -91"):
            list(stream)
        assert stream.summary is None
