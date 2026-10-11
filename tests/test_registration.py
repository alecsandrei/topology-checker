"""Dispatch, inference, and transaction coverage for automatic registration."""

from __future__ import annotations

import importlib.util
import json
from collections.abc import Iterator, Sequence
from concurrent.futures import ThreadPoolExecutor
from dataclasses import FrozenInstanceError
from typing import Any

import pytest
import shapely
from shapely.geometry import GeometryCollection, LineString, Point, mapping
from shapely.geometry.base import BaseGeometry
from topology_checker import GeometryKind, Rule, RuleSpec, Topology, _native

from .test_api import EDGE, INVALID_CASES, SQUARE, GeoObject


@pytest.mark.parametrize(
    "canonical,rule,data,other", INVALID_CASES, ids=[case[0] for case in INVALID_CASES]
)
def test_raw_dispatch_preserves_native_results(
    canonical: str,
    rule: Rule,
    data: Sequence[BaseGeometry],
    other: Sequence[BaseGeometry] | None,
) -> None:
    """Every concrete rule keeps counts, geometry, ordering, and provenance."""
    topology = Topology()
    spec = topology.add_rule(rule, data, other=other)
    assert spec.resolved_rule.value == canonical
    report = topology.validate()
    native = _native.NativeTopology(0)
    first = native.add_layer(
        "layer_0", [(i, shapely.to_wkb(g)) for i, g in enumerate(data)]
    )
    second = (
        None
        if other is None
        else native.add_layer(
            "layer_1", [(i, shapely.to_wkb(g)) for i, g in enumerate(other)]
        )
    )
    native.add_rule(canonical, first, second)
    expected: Any = json.loads(native.validate())
    assert report.rules[0].counts == {
        category: expected["rules"][0]["counts"][category.value]
        for category in report.rules[0].counts
    }
    assert report.error_count == len(expected["rules"][0]["errors"])
    for actual, error in zip(
        report.errors, expected["rules"][0]["errors"], strict=True
    ):
        assert actual.geometry.equals_exact(shapely.from_wkb(bytes(error["wkb"])), 0)
        assert actual.error_type.value == error["error_type"]
        assert actual.to_dict()["origin"] == error["origin"]
        assert actual.to_dict()["destination"] == error["destination"]
        assert actual.to_dict()["rule"] == rule.value
        assert actual.to_dict()["resolved_rule"] == canonical
    assert report.rules[0].to_dict()["resolved_rule"] == canonical
    if importlib.util.find_spec("geopandas") is None:
        with pytest.raises(ImportError, match="GeoDataFrame export requires"):
            report.to_geodataframe()
        return
    frame = report.to_geodataframe()
    assert frame["rule"].tolist() == [rule.value] * report.error_count
    assert frame["resolved_rule"].tolist() == [canonical] * report.error_count


@pytest.mark.parametrize(
    "data",
    [Point(0, 0), [Point(0, 0)], mapping(Point(0, 0)), GeoObject(Point(0, 0))],
)
def test_raw_input_adapters(data: Any) -> None:
    topology = Topology()
    spec = topology.add_rule(Rule.MUST_NOT_OVERLAP, data)
    assert topology.layer(spec.layer).kind is GeometryKind.POINT
    assert spec.layer.index == 0
    assert spec.other is None
    assert topology.validate().is_valid


def test_fresh_snapshots_explicit_reuse_and_mixed_forms() -> None:
    data = [Point(0, 0), Point(0, 0)]
    topology = Topology()
    first = topology.add_rule(Rule.MUST_NOT_OVERLAP, data)
    second = topology.add_rule(Rule.MUST_NOT_OVERLAP, data)
    assert first.layer != second.layer
    first_layers = topology.layers
    assert len(first_layers) == 2
    data[1] = Point(10, 10)
    third = topology.add_rule(Rule.MUST_NOT_OVERLAP, data, other=first.layer)
    fourth = topology.add_rule(Rule.MUST_NOT_OVERLAP, second.layer, other=data)
    shared = topology.add_rule(Rule.MUST_NOT_OVERLAP, first.layer)
    assert third.layer.index == 2 and third.other == first.layer
    assert fourth.layer == second.layer and fourth.other is not None
    assert fourth.other.index == 3
    assert shared.layer == first.layer and len(topology.layers) == 4
    assert [layer.name for layer in topology.layers] == [f"layer_{i}" for i in range(4)]
    assert topology.layer(first.layer).geometries[1] == Point(0, 0)
    assert isinstance(first, RuleSpec)
    report = topology.validate()
    assert [result.spec for result in report.rules] == [
        first,
        second,
        third,
        fourth,
        shared,
    ]
    with pytest.raises(FrozenInstanceError):
        first.layer = second.layer  # type: ignore[misc]
    with pytest.raises(FrozenInstanceError):
        first.resolved_rule.value = "changed"  # type: ignore[misc]
    with pytest.raises(FrozenInstanceError):
        topology.layer(first.layer).kind = GeometryKind.LINE  # type: ignore[misc]


def test_other_is_keyword_only_and_foreign_ids_are_rejected() -> None:
    topology = Topology()
    with pytest.raises(TypeError):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(0, 0)], [Point(0, 0)])  # type: ignore[call-arg]
    foreign = Topology().add_layer([Point(0, 0)])
    with pytest.raises(ValueError, match="belong"):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(0, 0)], other=foreign)
    with pytest.raises(ValueError, match="belong"):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, foreign, other=[Point(0, 0)])
    assert topology.layers == () and topology.validate().rules == ()


@pytest.mark.parametrize(
    "data,kind",
    [
        ([Point()], GeometryKind.POINT),
        ([LineString()], GeometryKind.LINE),
        ([shapely.from_wkt("MULTIPOLYGON EMPTY")], GeometryKind.POLYGON),
        (
            [shapely.from_wkt("GEOMETRYCOLLECTION (POINT EMPTY, MULTIPOINT EMPTY)")],
            GeometryKind.POINT,
        ),
        ([Point(), LineString()], None),
        ([GeometryCollection()], None),
        ([None, float("nan")], None),
        ([], None),
        (
            [GeometryCollection([Point(), GeometryCollection([EDGE])])],
            GeometryKind.LINE,
        ),
        ([GeometryCollection([Point(0, 0), EDGE])], GeometryKind.COLLECTION),
        ([Point(0, 0), SQUARE], GeometryKind.COLLECTION),
    ],
)
def test_recursive_family_inference(data: Any, kind: GeometryKind | None) -> None:
    topology = Topology()
    id = topology.add_layer(data)
    assert topology.layer(id).kind is kind


@pytest.mark.parametrize(
    "kind", [GeometryKind.POINT, GeometryKind.LINE, GeometryKind.POLYGON]
)
def test_declared_kinds_accept_empty_and_matching_parts(kind: GeometryKind) -> None:
    topology = Topology()
    # Declarations override empty hints, but never nonempty parts.
    id = topology.add_layer([Point()], kind=kind)
    assert topology.layer(id).kind is kind
    geometries = {
        GeometryKind.POINT: Point(1, 1),
        GeometryKind.LINE: EDGE,
        GeometryKind.POLYGON: SQUARE,
    }
    matching = topology.add_layer(
        [GeometryCollection([Point(), geometries[kind]])], kind=kind
    )
    assert topology.layer(matching).kind is kind
    with pytest.raises(ValueError, match="conflicts"):
        topology.add_layer(list(geometries.values()), kind=kind)
    assert len(topology.layers) == 2


@pytest.mark.parametrize(
    "kind", [GeometryKind.ANY, GeometryKind.COLLECTION, "point", 0]
)
def test_invalid_declarations_do_not_register(kind: Any) -> None:
    topology = Topology()
    with pytest.raises((ValueError, TypeError), match="kind"):
        topology.add_layer([], kind=kind)
    assert topology.layers == ()


def test_unknown_families_resolve_only_unique_candidates() -> None:
    topology = Topology()
    for rule, data, other in [
        (Rule.MUST_NOT_OVERLAP, [], None),
        (Rule.MUST_BE_INSIDE, [], [SQUARE]),
        (Rule.MUST_BE_COVERED_BY, [Point(1, 1)], []),
        (Rule.MUST_NOT_INTERSECT, [], []),
    ]:
        with pytest.raises(ValueError, match="ambiguous.*supported signatures.*kind="):
            topology.add_rule(rule, data, other=other)
    assert topology.layers == ()
    unique = topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, [])
    assert topology.layer(unique.layer).kind is None
    assert unique.resolved_rule.value == "line/must-not-have-dangles"
    partial = topology.add_rule(Rule.MUST_BE_INSIDE, [Point(1, 1)], other=[])
    assert partial.resolved_rule.value == "point/must-be-inside"
    typed = topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point()])
    assert typed.resolved_rule.value == "point/must-not-overlap"
    line = topology.add_layer([], kind=GeometryKind.LINE)
    constrained = topology.add_rule(Rule.MUST_BE_INSIDE, line, other=[])
    assert constrained.resolved_rule.value == "line/must-be-inside"
    report = topology.validate()
    assert report.rules[0].is_valid
    assert report.rules[1].error_count == 1
    assert all(result.is_valid for result in report.rules[2:])


@pytest.mark.parametrize(
    "rule,data,other",
    [
        (Rule.MUST_NOT_OVERLAP, [Point(0, 0)], [EDGE]),
        (Rule.MUST_NOT_OVERLAP, [SQUARE], [EDGE]),
        (Rule.MUST_BE_INSIDE, [SQUARE], [Point(0, 0)]),
        (Rule.MUST_NOT_INTERSECT, [EDGE], [Point(0, 0)]),
        (Rule.MUST_NOT_INTERSECT, [SQUARE], [EDGE]),
        (Rule.MUST_NOT_HAVE_DANGLES, [EDGE], [EDGE]),
        (Rule.MUST_CONTAIN, [SQUARE], None),
        (Rule.MUST_NOT_OVERLAP, [Point(0, 0), EDGE], None),
        (Rule.MUST_NOT_OVERLAP, [Point(0, 0)], [Point(0, 0), EDGE]),
    ],
)
def test_unsupported_ordered_signatures_are_atomic(
    rule: Rule, data: Any, other: Any
) -> None:
    topology = Topology()
    with pytest.raises(ValueError, match="unsupported.*supported signatures"):
        topology.add_rule(rule, data, other=other)
    assert topology.layers == () and topology.warnings() == ()


def test_mixed_families_use_only_any_signatures() -> None:
    mixed = GeometryCollection([Point(1, 1), EDGE])
    topology = Topology()
    unary = topology.add_rule(Rule.MUST_NOT_BE_MULTIPART, [mixed])
    binary = topology.add_rule(
        Rule.MUST_NOT_BE_CONTAINED_BY, unary.layer, other=[mixed]
    )
    assert topology.layer(unary.layer).kind is GeometryKind.COLLECTION
    assert binary.resolved_rule.inputs == (GeometryKind.ANY, GeometryKind.ANY)
    assert topology.validate().rules[0].error_count == 1


@pytest.mark.parametrize("side", [0, 1])
@pytest.mark.parametrize("bad", [object(), Point(float("inf"), 1)])
def test_failure_in_either_input_preserves_every_registration(
    side: int, bad: Any
) -> None:
    topology = Topology()
    existing = topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(2, 2), Point(2, 2)])
    before = topology.validate().to_dict()
    layers = topology.layers
    inputs: list[Any] = [[None, Point(0, 0)], [Point(0, 0)]]
    inputs[side] = [bad]
    with pytest.raises((TypeError, ValueError)):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, inputs[0], other=inputs[1])
    assert topology.layers == layers
    assert topology.validate().to_dict() == before
    assert topology.warnings() == tuple(before["warnings"])
    next_spec = topology.add_rule(
        Rule.MUST_NOT_OVERLAP, [Point(0, 0)], other=existing.layer
    )
    assert next_spec.layer.index == 1
    assert topology.layer(next_spec.layer).name == "layer_1"


def test_crs_conflicts_across_pending_and_existing_inputs_are_atomic() -> None:
    gpd = pytest.importorskip("geopandas")
    a = gpd.GeoSeries([Point(0, 0)], crs=4326)
    b = gpd.GeoSeries([Point(0, 0)], crs=3857)
    topology = Topology()
    with pytest.raises(ValueError, match="incompatible"):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, a, other=b)
    assert topology.layers == ()
    existing = topology.add_layer(a, name="points", fids=[99])
    before = topology.validate().to_dict()
    for primary, other in [(b, existing), (existing, b)]:
        with pytest.raises(ValueError, match="incompatible"):
            topology.add_rule(Rule.MUST_NOT_OVERLAP, primary, other=other)
        assert topology.validate().to_dict() == before
    spec = topology.add_rule(Rule.MUST_NOT_OVERLAP, a, other=existing)
    assert spec.layer.index == 1
    assert topology.layer(existing).fids == (99,)
    a.iloc[0] = Point(20, 20)
    assert topology.layer(spec.layer).geometries == (Point(0, 0),)
    assert topology.validate().crs == topology.layer(existing).crs


def test_concurrent_automatic_registration_has_contiguous_ids_and_specs() -> None:
    topology = Topology()

    def register(_: int) -> RuleSpec:
        return topology.add_rule(
            Rule.MUST_NOT_OVERLAP, [Point(0, 0)], other=[Point(0, 0)]
        )

    with ThreadPoolExecutor(max_workers=4) as pool:
        specs = list(pool.map(register, range(12)))
    assert [layer.id.index for layer in topology.layers] == list(range(24))
    report = topology.validate()
    assert sorted(spec.layer.index for spec in specs) == list(range(0, 24, 2))
    assert [result.spec.layer.index for result in report.rules] == list(range(0, 24, 2))
    for result in report.rules:
        assert result.other is not None and result.other.index == result.layer.index + 1
        error = result.errors[0]
        assert error.origin is not None and error.origin.layer == result.layer
        assert error.destination is not None and error.destination.layer == result.other


@pytest.mark.parametrize("side", [0, 1])
@pytest.mark.parametrize("failure", ["duplicates", "coordinates", "kind", "decode"])
def test_native_transaction_validates_before_commit(side: int, failure: str) -> None:
    """Native FID, coordinate, family, and WKB failures cannot leak state."""
    native = _native.NativeTopology(0)
    point = shapely.to_wkb(Point(0, 0))
    inputs: list[tuple[int | None, str, list[tuple[int, bytes]]]] = [
        (None, "primary", [(7, point)]),
        (None, "secondary", [(8, point)]),
    ]
    bad = {
        "duplicates": [(9, point), (9, point)],
        "coordinates": [(9, shapely.to_wkb(Point(float("inf"), 0)))],
        "kind": [(9, shapely.to_wkb(EDGE))],
        "decode": [(9, b"invalid")],
    }
    inputs[side] = (None, "bad", bad[failure])
    with pytest.raises(ValueError):
        native.register_rule("point/must-not-overlap-with", inputs)
    assert native.warnings() == []
    assert json.loads(native.validate())["rules"] == []
    assert native.register_rule(
        "point/must-not-overlap", [(None, "next", [(0, point)])]
    ) == (0, None)
    assert native.add_layer("after", []) == 1


def test_native_arity_and_same_id_failures_are_atomic() -> None:
    native = _native.NativeTopology(0)
    inputs: list[tuple[int | None, str, list[tuple[int, bytes]]]] = [(None, "new", [])]
    with pytest.raises(ValueError):
        native.register_rule("point/must-not-overlap-with", inputs)
    assert native.add_layer("existing", []) == 0
    invalid_inputs: list[list[tuple[int | None, str, list[tuple[int, bytes]]]]] = [
        [],
        inputs * 3,
        [(999, "", [])],
    ]
    for invalid in invalid_inputs:
        with pytest.raises(ValueError):
            native.register_rule("point/must-not-overlap", invalid)
    with pytest.raises(ValueError):
        native.register_rule("point/must-not-overlap-with", [(0, "", []), (0, "", [])])
    assert native.add_layer("after", []) == 1
    assert json.loads(native.validate())["rules"] == []


@pytest.mark.parametrize("outer_rule", [False, True])
@pytest.mark.parametrize("inner_rule", [False, True])
@pytest.mark.parametrize("form", ["iterator", "adapter"])
def test_input_callbacks_cannot_reenter_registration(
    outer_rule: bool, inner_rule: bool, form: str
) -> None:
    """Recursive adapter mutations fail without corrupting IDs or atomicity."""
    topology = Topology()
    original = topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(0, 0)])
    before = topology.validate().to_dict()

    def data() -> Sequence[BaseGeometry]:
        if inner_rule:
            topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(1, 1)])
        else:
            topology.add_layer([Point(1, 1)])
        return [Point(2, 2)]

    def iterator() -> Iterator[BaseGeometry]:
        yield from data()

    class Adapter:
        """Read geometry lazily through an input callback."""

        @property
        def __geo_interface__(self) -> dict[str, Any]:
            return mapping(GeometryCollection(data()))

    input_data = iterator() if form == "iterator" else Adapter()
    with pytest.raises(RuntimeError, match="reentered"):
        if outer_rule:
            topology.add_rule(Rule.MUST_NOT_OVERLAP, input_data)
        else:
            topology.add_layer(input_data)
    assert topology.validate().to_dict() == before
    assert topology.layer(original.layer).id.index == 0
    next_spec = topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(1, 1)])
    assert next_spec.layer.index == 1


def test_one_shot_iterators_are_consumed_by_each_raw_registration() -> None:
    """An exhausted generator is unknown empty data on its next use."""
    topology = Topology()
    data = iter([Point(0, 0), Point(0, 0)])
    first = topology.add_rule(Rule.MUST_NOT_OVERLAP, data)
    with pytest.raises(ValueError, match="ambiguous"):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, data)
    assert len(topology.layers) == 1
    topology.add_rule(Rule.MUST_NOT_OVERLAP, first.layer)
    assert topology.validate().error_count == 2
