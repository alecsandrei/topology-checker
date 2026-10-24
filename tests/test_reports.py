"""Per-rule report streams: eager equivalence, lifecycle, cancellation and errors."""

from __future__ import annotations

import gc
import json
import math
import os
import random
import threading
import time
import weakref
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any, cast

import pytest
import shapely
import topology_checker
from shapely.geometry import (
    GeometryCollection,
    LineString,
    MultiLineString,
    MultiPoint,
    MultiPolygon,
    Point,
    Polygon,
    box,
)
from shapely.geometry.base import BaseGeometry
from topology_checker import (
    GeometryKind,
    LayerId,
    PrecisionError,
    ReportStream,
    ReportSummary,
    Rule,
    RuleReport,
    Topology,
    _native,
)

VARIANTS = [(rule, variant) for rule in Rule for variant in rule.variants]
FAMILIES = (GeometryKind.POINT, GeometryKind.LINE, GeometryKind.POLYGON)
# Generous next to the milliseconds cancellation takes, and well below the
# seconds the heavy rule needs.
PROMPT = 2.0


def test_every_native_variant_is_generated() -> None:
    assert len(VARIANTS) == len(json.loads(_native.registry()))


@dataclass
class Stats:
    """What the generated topologies exercised, checked once over all seeds."""

    variants: set[str] = field(default_factory=lambda: set[str]())
    two_layer: int = 0
    repeated: int = 0
    empty_layers: int = 0
    warnings: int = 0
    passing: int = 0
    failing: int = 0
    no_rules: int = 0
    max_layers: int = 0


def geometry(rng: random.Random, kind: GeometryKind) -> BaseGeometry:
    """A small geometry on a coarse grid, so features often meet or overlap."""

    def xy() -> tuple[float, float]:
        return rng.randint(0, 6) / 2, rng.randint(0, 6) / 2

    if kind is GeometryKind.POINT:
        if rng.random() < 0.2:
            return MultiPoint([xy(), xy()])
        return Point(xy())
    if kind is GeometryKind.LINE:
        if rng.random() < 0.1:
            # Collapses to nothing: a degenerate part with a warning.
            x, y = xy()
            return MultiLineString([[(x, y), (x, y)], [xy(), xy()]])
        if rng.random() < 0.15:
            return MultiLineString([[xy(), xy()], [xy(), xy(), xy()]])
        return LineString([xy() for _ in range(rng.randint(2, 4))])
    if kind is GeometryKind.POLYGON:
        x, y = xy()
        w, h = rng.randint(1, 4) / 2, rng.randint(1, 4) / 2
        if rng.random() < 0.15:
            return MultiPolygon([box(x, y, x + w, y + h), box(x + 5, y, x + 6, y + 1)])
        if rng.random() < 0.15:
            return Polygon([(x, y), (x + w, y), (x, y + h)])
        return box(x, y, x + w, y + h)
    return GeometryCollection([Point(xy()), box(*xy(), 4, 4)])


def generated(seed: int, stats: Stats | None = None) -> Topology:
    """A seeded topology; the first rule cycles through every native variant."""
    rng = random.Random(seed)
    topology = Topology(rng.choice([0.0, 0.0, 0.25, 1.0]))
    layers: dict[GeometryKind, list[LayerId]] = {}

    def layer(kind: GeometryKind, avoid: LayerId | None = None) -> LayerId:
        if kind is GeometryKind.ANY:
            kind = rng.choice([*FAMILIES, GeometryKind.COLLECTION])
        known = [id for id in layers.get(kind, []) if id != avoid]
        if known and rng.random() < 0.5:
            return rng.choice(known)
        count = rng.choice([1, 2, 3, 5, 8])
        if kind is not GeometryKind.COLLECTION and rng.random() < 0.12:
            id = topology.add_layer([], kind=kind)
            if stats:
                stats.empty_layers += 1
        else:
            id = topology.add_layer(
                [geometry(rng, kind) for _ in range(count)],
                fids=rng.sample(range(-40, 40), count),
            )
        layers.setdefault(kind, []).append(id)
        return id

    if seed % 25 == 24:
        layer(rng.choice(FAMILIES))
        if stats:
            stats.no_rules += 1
        return topology
    for position in range(rng.randint(1, 4)):
        if position == 0:
            rule, variant = VARIANTS[seed % len(VARIANTS)]
        else:
            rule, variant = rng.choice(VARIANTS)
        primary, secondary = variant.inputs
        a = layer(primary)
        b = None if secondary is None else layer(secondary, avoid=a)
        spec = topology.add_rule(rule, a, other=b)
        assert spec.resolved_rule == variant
        if stats:
            stats.variants.add(variant.value)
            stats.two_layer += b is not None
        if rng.random() < 0.15:
            topology.add_rule(rule, a, other=b)
            if stats:
                stats.repeated += 1
    if stats:
        stats.max_layers = max(stats.max_layers, len(topology.layers))
    return topology


SEEDS = range(400)


@pytest.mark.parametrize("seed", SEEDS)
def test_reports_match_eager_rules_and_the_violation_stream(seed: int) -> None:
    topology = generated(seed)
    try:
        eager = topology.validate()
    except PrecisionError:
        with pytest.raises(PrecisionError):
            list(topology.iter_reports())
        with pytest.raises(PrecisionError):
            list(topology.iter_validate())
        return
    with topology.iter_reports() as stream:
        assert stream.summary is None
        reports = list(stream)
    assert reports == list(eager.rules)
    with topology.iter_validate(1 + seed % 5) as violations:
        streamed = tuple(violations)
    assert streamed == eager.violations
    assert streamed == tuple(v for report in reports for v in report.violations)
    summary = cast(ReportSummary | None, stream.summary)
    assert summary is not None
    assert summary == violations.summary
    assert (summary.tolerance, summary.warnings, summary.crs) == (
        eager.tolerance,
        eager.warnings,
        eager.crs,
    )
    assert [(r.spec, dict(r.counts), r.error_count) for r in summary.rules] == [
        (r.spec, dict(r.counts), r.error_count) for r in eager.rules
    ]
    assert (summary.error_count, summary.is_valid) == (
        eager.error_count,
        eager.is_valid,
    )


def test_generated_topologies_cover_the_interesting_shapes() -> None:
    stats = Stats()
    for seed in SEEDS:
        topology = generated(seed, stats)
        stats.warnings += bool(topology.warnings())
        for rule in topology.validate().rules:
            stats.passing += rule.is_valid
            stats.failing += not rule.is_valid
    assert stats.variants == {variant.value for _, variant in VARIANTS}
    assert min(stats.two_layer, stats.repeated, stats.empty_layers) >= 20
    assert min(stats.warnings, stats.no_rules, stats.passing, stats.failing) >= 10
    assert stats.max_layers >= 5


def mixed() -> Topology:
    """Three rules: one error, none, then two."""
    topology = Topology()
    topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(0, 0), Point(0, 0)])
    topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(0, 0), Point(1, 1)])
    topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, [LineString([(0, 0), (1, 0)])])
    return topology


def no_workers(timeout: float = 10.0) -> bool:
    """Wait until every validation worker thread has finished."""
    return _native.live_workers(timeout) == 0


def test_reports_are_exported_and_typed() -> None:
    assert "ReportStream" in topology_checker.__all__
    with mixed().iter_reports() as stream:
        assert isinstance(stream, ReportStream)
        assert iter(stream) is stream
        assert all(isinstance(report, RuleReport) for report in stream)
    assert [r.error_count for r in mixed().validate().rules] == [1, 0, 2]


def test_no_rules_yields_nothing_but_a_summary_with_warnings() -> None:
    topology = Topology()
    topology.add_layer([MultiLineString([[(0, 0), (0, 0)], [(0, 0), (1, 0)]])])
    with topology.iter_reports() as stream:
        assert list(stream) == []
    assert stream.summary is not None
    assert stream.summary.rules == ()
    assert stream.summary.warnings == topology.warnings() != ()
    assert stream.summary.warnings == topology.validate().warnings


def test_partial_iteration_then_close() -> None:
    topology = mixed()
    eager = topology.validate()
    stream = topology.iter_reports()
    assert next(stream) == eager.rules[0]
    stream.close()
    stream.close()
    assert stream.summary is None
    assert list(stream) == []
    with pytest.raises(StopIteration):
        next(stream)
    assert no_workers()
    # A finished stream stays finished and keeps its summary.
    with topology.iter_reports() as stream:
        assert list(stream) == list(eager.rules)
    stream.close()
    assert stream.summary is not None and list(stream) == []


def test_break_and_consumer_errors_close_the_stream() -> None:
    topology = mixed()
    with topology.iter_reports() as stream:
        for _ in stream:
            break
    assert stream.summary is None and list(stream) == []
    with pytest.raises(LookupError, match="consumer"):
        with topology.iter_reports() as stream:
            next(stream)
            raise LookupError("consumer")
    assert stream.summary is None and list(stream) == []
    assert no_workers()


def test_dropping_an_unclosed_stream_stops_its_worker() -> None:
    topology = mixed()
    started = topology.iter_reports()
    next(started)
    del started
    never_started = topology.iter_reports()
    del never_started
    gc.collect()
    assert no_workers()


def test_stream_outlives_its_topology() -> None:
    topology = mixed()
    expected = list(topology.validate().rules)
    stream = topology.iter_reports()
    alive = weakref.ref(topology)
    del topology
    gc.collect()
    assert alive() is None
    assert list(stream) == expected
    assert stream.summary is not None and stream.summary.error_count == 3


def test_interleaved_streams_from_one_topology() -> None:
    topology = mixed()
    eager = topology.validate()
    first, second = topology.iter_reports(), topology.iter_reports()
    violations = topology.iter_validate(1)
    seen: list[list[RuleReport]] = [[], []]
    for _ in eager.rules:
        seen[1].append(next(second))
        seen[0].append(next(first))
    assert next(violations) == eager.violations[0]
    assert seen == [list(eager.rules)] * 2
    assert list(first) == list(second) == []
    assert first.summary is not None and first.summary == second.summary
    assert list(violations) == list(eager.violations[1:])
    assert violations.summary == first.summary


def in_thread(work: Callable[[], Any]) -> Any:
    """Run work on another Python thread and return or raise its outcome."""
    outcome: list[Any] = []
    failure: list[BaseException] = []

    def run() -> None:
        try:
            outcome.append(work())
        except BaseException as error:
            failure.append(error)

    thread = threading.Thread(target=run)
    thread.start()
    thread.join()
    if failure:
        raise failure[0]
    return outcome[0]


def test_streams_work_on_other_threads() -> None:
    topology = mixed()
    expected = list(topology.validate().rules)
    stream = topology.iter_reports()
    assert in_thread(lambda: list(stream)) == expected
    assert stream.summary is not None

    def own_stream() -> list[RuleReport]:
        with topology.iter_reports() as stream:
            return list(stream)

    assert in_thread(own_stream) == expected
    partial = topology.iter_reports()
    assert in_thread(lambda: next(partial)) == expected[0]
    assert next(partial) == expected[1]
    partial.close()
    assert no_workers()


def open_descriptors() -> int | None:
    for folder in ("/proc/self/fd", "/dev/fd"):
        if os.path.isdir(folder):
            return len(os.listdir(folder))
    return None


def test_many_streams_leave_no_threads_or_descriptors() -> None:
    topology = mixed()
    expected = list(topology.validate().rules)
    gc.collect()
    assert no_workers()
    threads, descriptors = threading.active_count(), open_descriptors()
    for i in range(200):
        stream = topology.iter_reports()
        match i % 4:
            case 0:
                assert list(stream) == expected
            case 1:
                next(stream)
                stream.close()
            case 2:
                next(stream)
            case _:
                with stream:
                    pass
        del stream
    gc.collect()
    assert no_workers()
    assert threading.active_count() == threads
    assert open_descriptors() == descriptors


def test_consumed_reports_are_not_retained() -> None:
    topology = mixed()
    stream = topology.iter_reports()
    report = next(stream)
    refs = [weakref.ref(report), weakref.ref(report.violations[0])]
    del report
    refs += [weakref.ref(report) for report in stream]
    gc.collect()
    assert [ref() for ref in refs] == [None] * 4
    assert stream.summary is not None


def test_registrations_after_opening_do_not_change_the_stream() -> None:
    topology = mixed()
    expected = list(topology.validate().rules)
    stream = topology.iter_reports()
    topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(5, 5)] * 3)
    assert next(stream) == expected[0]
    layer = topology.add_layer([Point(0, 0)] * 2)
    topology.add_rule(Rule.MUST_NOT_OVERLAP, layer)
    assert [next(stream), next(stream)] == expected[1:]
    with pytest.raises(StopIteration):
        next(stream)
    assert stream.summary is not None and len(stream.summary.rules) == 3
    assert len(topology.validate().rules) == 5


def narrow() -> list[Polygon]:
    """Gap output that cannot be emitted with bounded coordinate rounding."""
    x = math.nextafter(1.0, 2.0)
    return [
        Polygon([(1, 0), (x, 0), (x, 1), (1, 1)]),
        Polygon([(1, -1), (x, -1), (1, 1)]),
    ]


def test_first_report_arrives_before_a_later_rule_fails() -> None:
    topology = Topology()
    topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(0, 0), Point(0, 0)])
    topology.add_rule(Rule.MUST_NOT_HAVE_GAPS, narrow())
    with pytest.raises(PrecisionError):
        topology.validate()
    # validate() fails, yet the first rule's report is already available.
    with topology.iter_reports() as stream:
        first = next(stream)
    assert first.error_count == 1 and first.counts
    assert stream.summary is None


def test_failure_after_reports_keeps_them_and_raises_at_the_failing_rule() -> None:
    topology = Topology()
    points = topology.add_layer([Point(0, 0), Point(0, 0)], fids=[3, -8])
    topology.add_rule(Rule.MUST_NOT_OVERLAP, points)
    topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, [LineString([(0, 0), (1, 0)])])
    expected = [report.to_dict() for report in topology.validate().rules]
    topology.add_rule(Rule.MUST_NOT_HAVE_GAPS, narrow())
    topology.add_rule(Rule.MUST_NOT_OVERLAP, points)
    stream = topology.iter_reports()
    reports = [next(stream), next(stream)]
    with pytest.raises(PrecisionError):
        next(stream)
    assert stream.summary is None
    assert list(stream) == []
    assert [report.to_dict() for report in reports] == expected
    assert reports[0].violations[0].origin is not None
    assert {reports[0].violations[0].origin.fid} <= {3, -8}
    with topology.iter_validate(1) as violations:
        assert len([next(violations) for _ in range(3)]) == 3
        with pytest.raises(PrecisionError):
            next(violations)
    assert violations.summary is None
    failing_first = Topology()
    failing_first.add_rule(Rule.MUST_NOT_HAVE_GAPS, narrow())
    failing_first.add_rule(Rule.MUST_NOT_OVERLAP, [Point(0, 0)] * 2)
    stream = failing_first.iter_reports()
    with pytest.raises(PrecisionError):
        next(stream)
    assert stream.summary is None and list(stream) == []
    assert no_workers()


@pytest.fixture(scope="module")
def grid() -> Any:
    """250 000 touching squares: seconds of overlap checking, no violations."""
    x = [float(k % 500) for k in range(250_000)]
    y = [float(k // 500) for k in range(250_000)]
    return shapely.box(x, y, [v + 1 for v in x], [v + 1 for v in y])


def heavy(grid: Any, cheap_first: bool) -> Topology:
    topology = Topology()
    if cheap_first:
        topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(0, 0), Point(1, 1)])
    topology.add_rule(Rule.MUST_NOT_OVERLAP, grid)
    return topology


def test_close_during_a_long_rule_is_prompt(grid: Any) -> None:
    stream = heavy(grid, cheap_first=True).iter_reports()
    assert next(stream).is_valid
    started = time.monotonic()
    stream.close()
    assert time.monotonic() - started < PROMPT
    assert stream.summary is None and list(stream) == []
    assert no_workers(0)


def test_dropping_a_stream_during_a_long_rule_stops_its_worker(grid: Any) -> None:
    stream = heavy(grid, cheap_first=True).iter_reports()
    next(stream)
    del stream
    gc.collect()
    assert no_workers(PROMPT)


def test_close_from_another_thread_wakes_a_waiting_consumer(grid: Any) -> None:
    stream = heavy(grid, cheap_first=False).iter_reports()
    waiting = threading.Event()
    outcome: list[object] = []

    def consume() -> None:
        waiting.set()
        outcome.append(next(stream, "stopped"))

    consumer = threading.Thread(target=consume)
    consumer.start()
    waiting.wait()
    started = time.monotonic()
    stream.close()
    consumer.join(PROMPT)
    assert time.monotonic() - started < PROMPT
    assert not consumer.is_alive()
    assert outcome == ["stopped"]
    assert stream.summary is None
    assert no_workers(0)


def test_close_while_another_thread_holds_the_gil(grid: Any) -> None:
    stop = threading.Event()
    spins = [0]

    def spin() -> None:
        while not stop.is_set():
            spins[0] += 1

    spinner = threading.Thread(target=spin)
    spinner.start()
    try:
        stream = heavy(grid, cheap_first=True).iter_reports()
        next(stream)
        # This thread still runs Python while the worker checks the grid.
        before = spins[0]
        while spins[0] < before + 1000:
            pass
        started = time.monotonic()
        stream.close()
        assert time.monotonic() - started < PROMPT
    finally:
        stop.set()
        spinner.join()
    assert stream.summary is None
    assert no_workers(0)
