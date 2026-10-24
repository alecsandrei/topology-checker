"""What a run tells as it goes, with each engine: the stage it is at, and
each rule as soon as it is done, with its errors where the engine has them.
A run that tells all this writes the same errors as one that did not."""

from __future__ import annotations

import collections.abc as c
import contextlib
import sqlite3
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsGeometry, QgsProcessingFeedback

from topology_rules.core import checks, commands, engine, package
from topology_rules.core.binary import Rule
from topology_rules.core.checks import Job
from topology_rules.core.engine import Package, Tool
from topology_rules.core.layers import Input
from topology_rules.core.progress import Checked, Stage, counted, label
from topology_rules.exceptions import RunError

from .helpers import line_layer, point_layer, polygon_layer
from .live import (
    Recorder,
    expected_stages,
    forwarding_executable,
    found_rows,
    hold_reports,
    run_told,
    run_validate_untold,
    validate_untold,
)
from .test_engines_agree import (
    BOWTIE,
    CHAIN,
    SCATTERED,
    SHIFTED,
    SQUARE,
    project,
    same_tables,
    tables,
    wkt,
)

FAR_SQUARE = 'POLYGON((40 0, 50 0, 50 10, 40 10, 40 0))'
RING = 'LINESTRING(0 0, 10 0, 10 10, 0 0)'
CROSSING = 'LINESTRING(0 -5, 5 5)'

Spec = tuple[str, Input] | tuple[str, Input, Input]


def lines() -> list[Spec]:
    roads = project(line_layer(*CHAIN, CROSSING, name='roads'))
    return [
        ('line/must-not-have-dangles', roads),
        ('line/must-not-intersect', roads),
        ('line/must-not-self-intersect', roads),
    ]


def polygons() -> list[Spec]:
    parcels = project(
        polygon_layer(SQUARE, SHIFTED, FAR_SQUARE, name='parcels')
    )
    return [
        ('polygon/must-not-overlap', parcels),
        ('polygon/must-not-have-gaps', parcels),
        ('geometry/must-not-be-multipart', parcels),
    ]


def points() -> list[Spec]:
    wells = project(
        point_layer(
            'POINT(0 0)',
            'POINT(0 0)',
            'POINT(5 5)',
            'POINT(10 0)',
            name='wells',
        )
    )
    roads = project(line_layer(*CHAIN, name='roads'))
    return [
        ('point/must-not-overlap', wells),
        ('point/must-be-covered-by-lines', wells, roads),
        ('point/must-be-covered-by-line-endpoints', wells, roads),
    ]


def two_layers() -> list[Spec]:
    roads = project(line_layer(*CHAIN, name='roads'))
    rivers = project(line_layer('LINESTRING(5 -5, 5 5)', name='rivers'))
    parcels = project(polygon_layer(SQUARE, FAR_SQUARE, name='parcels'))
    wells = project(point_layer('POINT(5 5)', 'POINT(70 70)', name='wells'))
    return [
        ('line/must-not-intersect-with', roads, rivers),
        ('polygon/must-contain-point', parcels, wells),
        ('line/must-be-inside', roads, parcels),
    ]


def invalid() -> list[Spec]:
    """A polygon that crosses itself, which both engines leave out and
    report, and errors of the others."""
    parcels = project(polygon_layer(SQUARE, SHIFTED, BOWTIE, name='parcels'))
    roads = project(line_layer(*CHAIN, name='roads'))
    return [
        ('polygon/must-not-overlap', parcels),
        ('line/must-not-have-dangles', roads),
    ]


def zero_in_the_middle() -> list[Spec]:
    roads = project(line_layer(*CHAIN, CROSSING, name='roads'))
    return [
        ('line/must-not-have-dangles', roads),
        ('line/must-not-overlap', roads),
        ('line/must-not-intersect', roads),
    ]


def all_zero() -> list[Spec]:
    ring = project(line_layer(RING, name='ring'))
    return [
        ('line/must-not-have-dangles', ring),
        ('line/must-not-intersect', ring),
    ]


def single() -> list[Spec]:
    return [('line/must-not-have-dangles', project(line_layer(*CHAIN)))]


def families() -> list[Spec]:
    """Rules whose errors are points, lines and polygons, in turn."""
    roads = project(line_layer(*CHAIN, 'LINESTRING(0 0, 10 0)', name='roads'))
    parcels = project(polygon_layer(SQUARE, SHIFTED, name='parcels'))
    return [
        ('line/must-not-have-dangles', roads),
        ('polygon/must-not-overlap', parcels),
        ('line/must-not-overlap', roads),
        ('line/must-not-have-dangles', roads),
    ]


def many() -> list[Spec]:
    scattered = project(line_layer(*SCATTERED, name='scattered'))
    return [
        ('line/must-not-have-dangles', scattered),
        ('line/must-not-intersect', scattered),
    ]


SHAPES: dict[str, c.Callable[[], list[Spec]]] = {
    'lines': lines,
    'polygons': polygons,
    'points': points,
    'two-layers': two_layers,
    'invalid': invalid,
    'zero-in-the-middle': zero_in_the_middle,
    'all-zero': all_zero,
    'single': single,
    'families': families,
    'many': many,
}


@pytest.fixture(params=['executable', 'package'])
def tool(request: pytest.FixtureRequest, executable: str) -> c.Iterator[Tool]:
    """Each engine in turn."""
    if request.param == 'package':
        yield request.getfixturevalue('package')
    else:
        yield executable


def jobs_of(tool: Tool, specs: list[Spec]) -> list[Job]:
    rules: dict[str, Rule] = {rule.name: rule for rule in engine.rules(tool)}
    return [Job(rules[name], *inputs) for name, *inputs in specs]


def last_attempt(recorder: Recorder) -> list[object]:
    """What the run told from the last time it read the layers: a tool that
    rejects a feature is run again without it."""
    events = list(recorder.events)
    starts = [
        i
        for i, event in enumerate(events)
        if not isinstance(event, Checked) and event[0] is Stage.READING
    ]
    assert starts, events
    return events[starts[-1] :]


def told_in_order(recorder: Recorder, total: int, executable: bool) -> None:
    """A run reads, then checks the rules one after the other, telling of
    each when it is done and which comes next, then writes the errors."""
    events = last_attempt(recorder)
    stages = expected_stages(total, executable)
    expected: list[object] = [stages[0], stages[1]]
    done = [e for e in events if isinstance(e, Checked)]
    assert [d.index for d in done] == list(range(total))
    for index in range(total):
        expected += [done[index], stages[index + 2]]
    assert events == expected


def file_rows(path: Path) -> dict[str, list[list[object]]]:
    """The errors of the GeoPackage by family, in the order of the file, as
    the geometry and the columns."""
    if not path.exists():
        return {}
    found: dict[str, list[list[object]]] = {}
    with contextlib.closing(sqlite3.connect(path)) as db:
        names = [
            name
            for (name,) in db.execute(
                'select table_name from gpkg_geometry_columns'
            )
        ]
        for name in sorted(names):
            rows = db.execute(
                'select geom, violation_id, rule, error_type, origin_layer, '
                'origin_fid, origin_part, destination_layer, '
                f'destination_fid, destination_part from "{name}" order by fid'
            )
            found[name.removeprefix('errors_')] = [
                [wkt(geometry), *values] for geometry, *values in rows
            ]
    return found


def rule_lines(log: str, jobs: list[Job]) -> list[str]:
    labels = {label(job) for job in jobs}
    return [
        line for line in log.splitlines() if line.rpartition(': ')[0] in labels
    ]


@pytest.mark.parametrize('shape', list(SHAPES))
def test_each_rule_is_told_as_soon_as_it_is_done(
    tool: Tool, shape: str, tmp_path: Path
):
    jobs = jobs_of(tool, SHAPES[shape]())
    told = run_told(tool, jobs, tmp_path / 'run')
    executable = not isinstance(tool, Package)
    told_in_order(told.feedback, len(jobs), executable)
    done = told.feedback.checks()[-len(jobs) :]
    assert [d.count for d in done] == told.outcome.counts
    assert {d.total for d in done} == {len(jobs)}
    for d in done:
        assert dict(d.layers) == told.outcome.layers
    if executable:
        # The errors are in the file once the tool is done with all rules
        assert all(d.errors is None for d in done)
    else:
        assert all(d.errors is not None for d in done)
        assert found_rows(done) == file_rows(told.output)
    # The log has a line for each rule, in the order of the rules
    assert rule_lines(told.feedback.textLog(), jobs) == [
        f'{label(job)}: {counted(count)}'
        for job, count in zip(jobs, told.outcome.counts, strict=True)
    ]


def test_the_errors_of_a_rule_are_in_the_order_of_their_layers(
    package: Package, tmp_path: Path
):
    """By family, as the panel reads them from the layers, then as they
    were found."""
    jobs = jobs_of(package, families())
    told = run_told(package, jobs, tmp_path / 'run')
    order = ['point', 'line', 'polygon']
    for done in told.feedback.checks():
        kinds = [order.index(found.family) for found in done.errors or ()]
        assert kinds == sorted(kinds)
        for found in done.errors or ():
            assert QgsGeometry(found.geometry).isMultipart()


def test_a_rule_is_told_with_the_names_of_its_layers(
    package: Package, tmp_path: Path
):
    jobs = jobs_of(package, two_layers())
    told = run_told(package, jobs, tmp_path / 'run')
    first = told.feedback.checks()[0]
    names = {found.values[3] for found in first.errors or ()} | {
        found.values[6] for found in first.errors or ()
    }
    assert names <= {'roads', 'rivers', None}
    assert set(first.layers) == {'roads', 'rivers', 'parcels', 'wells'}


def test_the_executable_is_asked_to_tell_of_each_rule(
    executable: str, tmp_path: Path
):
    log = tmp_path / 'commands'
    forwarding = str(forwarding_executable(tmp_path, log))
    jobs = jobs_of(executable, lines())
    told = run_told(forwarding, jobs, tmp_path / 'run')
    validate = [
        line for line in log.read_text().splitlines() if 'validate' in line
    ]
    assert len(validate) == 1
    assert '--progress' in validate[0].split()
    assert len(told.feedback.checks()) == len(jobs)


def test_a_run_of_the_toolbox_is_not_told(executable: str, tmp_path: Path):
    """A feedback that is not the panel's gets the log and the progress."""
    jobs = jobs_of(executable, lines())
    feedback = QgsProcessingFeedback()
    told = run_told(executable, jobs, tmp_path / 'run', feedback=None)
    plain = run_told(
        executable, jobs, tmp_path / 'plain', feedback=t.cast(t.Any, feedback)
    )
    assert plain.outcome.counts == told.outcome.counts
    assert rule_lines(feedback.textLog(), jobs) == rule_lines(
        told.feedback.textLog(), jobs
    )


def warnings_of(log: str) -> list[str]:
    return sorted({line for line in log.splitlines() if 'warning' in line})


@pytest.mark.parametrize('shape', list(SHAPES))
def test_a_told_run_writes_what_a_run_did_before(
    tool: Tool,
    shape: str,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
):
    jobs = jobs_of(tool, SHAPES[shape]())
    told = run_told(tool, jobs, tmp_path / 'told')
    with monkeypatch.context() as patched:
        patched.setattr(package, 'validate', validate_untold)
        patched.setattr(checks, 'run_validate', run_validate_untold)
        before = run_told(
            tool,
            jobs,
            tmp_path / 'before',
            feedback=t.cast(t.Any, QgsProcessingFeedback()),
        )
    first = (
        tables(told.output, told.outcome.ids) if told.output.exists() else None
    )
    second = (
        tables(before.output, before.outcome.ids)
        if before.output.exists()
        else None
    )
    same_tables(first, second)
    assert file_rows(told.output) == file_rows(before.output)
    assert told.outcome.counts == before.outcome.counts
    assert told.outcome.ids == before.outcome.ids
    assert told.outcome.layers == before.outcome.layers
    assert [(i.alias, i.fid, i.reason) for i in told.outcome.invalid] == [
        (i.alias, i.fid, i.reason) for i in before.outcome.invalid
    ]
    assert warnings_of(told.feedback.textLog()) == warnings_of(
        t.cast(t.Any, before.feedback).textLog()
    )


def test_the_shape_with_invalid_features_has_some(tool: Tool, tmp_path: Path):
    """The test of invalid rows needs invalid rows."""
    jobs = jobs_of(tool, invalid())
    told = run_told(tool, jobs, tmp_path / 'run')
    assert told.outcome.invalid


def test_the_shapes_have_what_they_are_named_after(
    package: Package, tmp_path: Path
):
    counts = {
        shape: run_told(
            package, jobs_of(package, make()), tmp_path / shape
        ).outcome.counts
        for shape, make in SHAPES.items()
    }
    assert counts['zero-in-the-middle'][1] == 0
    assert counts['zero-in-the-middle'][0] > 0
    assert counts['zero-in-the-middle'][2] > 0
    assert counts['all-zero'] == [0, 0]
    assert len(counts['single']) == 1
    assert all(counts['families'])
    assert sum(counts['many']) >= 80


# Failures


@pytest.mark.parametrize('after', [0, 1, 2])
def test_the_package_failing_after_some_rules_tells_of_those(
    package: Package,
    after: int,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
):
    gate = hold_reports(monkeypatch)[0]
    gate.fail_at = after
    gate.failure = 'the package ran out of memory'
    gate.let_all()
    jobs = jobs_of(package, lines())
    feedback = Recorder()
    with pytest.raises(RunError, match='ran out of memory'):
        run_told(package, jobs, tmp_path / 'run', feedback=feedback)
    assert [d.index for d in feedback.checks()] == list(range(after))
    assert len(rule_lines(feedback.textLog(), jobs)) == after
    assert (Stage.WRITING, None, len(jobs)) not in feedback.stages()


def test_progress_lines_of_another_run_are_not_told():
    """A line that counts rules otherwise than the run does says nothing of
    this run."""
    told: list[tuple[int, int, int]] = []
    lines = commands.follower(lambda *found: told.append(found))
    assert lines('checked rule 2/3 (line/must-not-overlap): 4 errors')
    assert not lines('checked rule 0/3 (line/must-not-overlap): 4 errors')
    assert not lines('checked rule 4/3 (line/must-not-overlap): 4 errors')
    assert told == [(1, 3, 4)]
