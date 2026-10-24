"""Runs from the Processing toolbox and from scripts, with no panel: each rule
says its count in the log as soon as it is done, a canceled run ends as
the toolbox expects, and the outputs are those of runs from before the
engines told of each rule. A batch, or runs at the same time, keep to
their own rules."""

from __future__ import annotations

import collections.abc as c
import dataclasses
import json
import typing as t
from pathlib import Path

import pytest
from qgis.core import (
    QgsApplication,
    QgsProcessingAlgRunnerTask,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
    QgsVariantUtils,
    QgsVectorLayer,
)

from topology_rules.algorithms.check import COUNTS
from topology_rules.algorithms.rule import COUNT
from topology_rules.algorithms.sinks import SINKS
from topology_rules.core import engine
from topology_rules.core.binary import Rule
from topology_rules.core.checks import Job
from topology_rules.core.engine import Engine
from topology_rules.core.layers import Input
from topology_rules.core.progress import counted, label
from topology_rules.enums import PROVIDER_ID
from topology_rules.utils import not_none

from .helpers import (
    Ran,
    Started,
    errors_layer,
    line_layer,
    point_layer,
    polygon_layer,
    run_algorithm,
    start_algorithm,
    wait_for,
)
from .live import (
    Gates,
    Hold,
    close_gates,
    hold_reports,
    hold_writing,
    let_all_lines,
    let_end,
    let_lines,
    replaying_executable,
    untold,
)
from .test_engines_agree import BOWTIE, CHAIN, SHIFTED, SQUARE
from .test_live_engines import file_rows

CROSSING = 'LINESTRING(0 -5, 5 5)'
RING = 'LINESTRING(0 0, 10 0, 10 10, 0 0)'


@pytest.fixture(autouse=True)
def ready(run_engine: Engine, executable: str, provider: object) -> None:
    """The real executable and the provider, and each engine in turn."""


def project_layer(layer: QgsVectorLayer) -> QgsVectorLayer:
    QgsProject.instance().addMapLayer(layer)
    return layer


def roads() -> QgsVectorLayer:
    return project_layer(line_layer(*CHAIN, CROSSING, name='roads'))


def parcels(*extra: str) -> QgsVectorLayer:
    return project_layer(polygon_layer(SQUARE, SHIFTED, *extra, name='parcels'))


# A rule of the toolbox, on its layers
Case = tuple[str, QgsVectorLayer, QgsVectorLayer | None]


def line_rule() -> Case:
    return ('line/must-not-have-dangles', roads(), None)


def polygon_rule() -> Case:
    return ('polygon/must-not-overlap', parcels(), None)


def point_rule() -> Case:
    wells = project_layer(
        point_layer('POINT(5 0)', 'POINT(5 30)', name='wells')
    )
    return ('point/must-be-covered-by-lines', wells, roads())


def two_layer_rule() -> Case:
    rivers = project_layer(line_layer('LINESTRING(5 -5, 5 5)', name='rivers'))
    return ('line/must-not-intersect-with', roads(), rivers)


def invalid_rule() -> Case:
    return ('polygon/must-not-overlap', parcels(BOWTIE), None)


def zero_rule() -> Case:
    return (
        'line/must-not-have-dangles',
        project_layer(line_layer(RING, name='ring')),
        None,
    )


CASES: dict[str, c.Callable[[], Case]] = {
    'line': line_rule,
    'polygon': polygon_rule,
    'point': point_rule,
    'two-layers': two_layer_rule,
    'invalid': invalid_rule,
    'zero': zero_rule,
}


def rule_of(name: str) -> Rule:
    rules = engine.rules(engine.tool())
    return next(rule for rule in rules if rule.name == name)


def parameters_of(case: Case) -> dict[str, t.Any]:
    _, layer, other = case
    parameters: dict[str, t.Any] = {'INPUT': layer.id()}
    if other is not None:
        parameters['OTHER'] = other.id()
    for sink, *_ in SINKS:
        parameters[sink] = 'TEMPORARY_OUTPUT'
    return parameters


def rule_line(case: Case, count: int) -> str:
    """The line of the log of a rule that is done."""
    name, layer, other = case
    job = Job(
        rule_of(name),
        Input(layer.name(), layer, layer),
        None if other is None else Input(other.name(), other, other),
    )
    return f'{label(job)}: {counted(count)}'


def ruled(log: str) -> list[str]:
    """The lines of the log that tell of a rule that is done."""
    return [
        line
        for line in log.splitlines()
        if line.endswith((' errors', ' error'))
        and not line.endswith('topology errors')
        and ': ' in line
    ]


Sink = list[tuple[tuple[object, ...], str]]


def sink_layers(ran: Ran) -> dict[str, QgsVectorLayer]:
    return {
        sink: errors_layer(ran, sink)
        for sink, *_ in SINKS
        if sink in ran.results
    }


def sinks_of(ran: Ran) -> dict[str, Sink]:
    """The features of each output of errors, in order."""
    return {
        sink: [
            (tuple(feature.attributes()), feature.geometry().asWkt(6))
            for feature in not_none(layer.dataProvider()).getFeatures()
        ]
        for sink, layer in sink_layers(ran).items()
    }


def violations(ran: Ran) -> int:
    """The errors that the rules found, which the features that they skipped
    are not: those have no number."""
    return sum(
        not QgsVariantUtils.isNull(feature['violation_id'])
        for layer in sink_layers(ran).values()
        for feature in not_none(layer.dataProvider()).getFeatures()
    )


def run_with(
    name: str, parameters: dict[str, t.Any], feedback: QgsProcessingFeedback
) -> Ran:
    """Runs an algorithm in a task, as a batch does, with its feedback."""
    algorithm = not_none(
        QgsApplication.processingRegistry().createAlgorithmById(
            f'{PROVIDER_ID}:{name}'
        )
    )
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    task = QgsProcessingAlgRunnerTask(algorithm, parameters, context, feedback)
    outcome: list[tuple[bool, dict[str, t.Any]]] = []
    task.executed.connect(lambda ok, results: outcome.append((ok, results)))
    QgsApplication.taskManager().addTask(task)
    wait_for(lambda: bool(outcome))
    ok, results = outcome[0]
    return Ran(results, ok, context, feedback.textLog())


# The rules of the toolbox


@pytest.mark.parametrize('case', list(CASES))
def test_a_rule_says_its_count_in_the_log(case: str):
    made = CASES[case]()
    ran = run_algorithm(rule_of(made[0]).slug, parameters_of(made))
    assert ran.ok, ran.log
    assert ruled(ran.log) == [rule_line(made, violations(ran))]


@pytest.mark.parametrize('case', list(CASES))
def test_a_rule_writes_what_it_wrote_before(
    case: str, monkeypatch: pytest.MonkeyPatch
):
    made = CASES[case]()
    algorithm = rule_of(made[0]).slug
    told = run_algorithm(algorithm, parameters_of(made))
    with monkeypatch.context() as patched:
        untold(patched)
        before = run_algorithm(algorithm, parameters_of(made))
    assert told.ok
    assert before.ok
    # The run from before told of no rule
    assert ruled(before.log) == []
    assert told.results[COUNT] == before.results[COUNT]
    assert sinks_of(told) == sinks_of(before)
    if case == 'zero':
        assert told.results[COUNT] == 0
    if case == 'invalid':
        assert told.results[COUNT] > 1


def test_a_batch_of_rules_runs_as_the_rules_one_by_one():
    """A batch runs each row after the other, with one feedback: each row
    writes what it writes alone, and the log tells of each row's rule."""
    made = [CASES[case]() for case in ('line', 'polygon', 'two-layers')]
    alone = [
        run_algorithm(rule_of(case[0]).slug, parameters_of(case))
        for case in made
    ]
    feedback = QgsProcessingFeedback()
    batch = [
        run_with(rule_of(case[0]).slug, parameters_of(case), feedback)
        for case in made
    ]
    assert [ran.ok for ran in batch] == [True] * len(made)
    assert [sinks_of(ran) for ran in batch] == [sinks_of(ran) for ran in alone]
    # One log for the batch, with the line of each rule in turn
    assert ruled(feedback.textLog()) == [
        line for ran in alone for line in ruled(ran.log)
    ]


def test_runs_at_the_same_time_keep_to_their_own_rules(tmp_path: Path):
    """Two runs of the rules of the panel at once, as from two scripts."""
    lines, polygons = roads(), parcels()
    first = {
        'RULES': json.dumps(
            [
                {'rule': 'line/must-not-have-dangles', 'layer': lines.id()},
                {'rule': 'line/must-not-intersect', 'layer': lines.id()},
            ]
        ),
        'OUTPUT': str(tmp_path / 'first.gpkg'),
    }
    second = {
        'RULES': json.dumps(
            [{'rule': 'polygon/must-not-overlap', 'layer': polygons.id()}]
        ),
        'OUTPUT': str(tmp_path / 'second.gpkg'),
    }
    together = [
        start_algorithm('check_rules', first),
        start_algorithm('check_rules', second),
    ]
    ran = [started.ran() for started in together]
    alone = [
        run_algorithm(
            'check_rules', {**first, 'OUTPUT': str(tmp_path / 'a.gpkg')}
        ),
        run_algorithm(
            'check_rules', {**second, 'OUTPUT': str(tmp_path / 'b.gpkg')}
        ),
    ]
    assert [r.ok for r in ran] == [True, True]
    assert [r.results[COUNTS] for r in ran] == [
        r.results[COUNTS] for r in alone
    ]
    assert [file_rows(Path(r.results['OUTPUT'])) for r in ran] == [
        file_rows(Path(r.results['OUTPUT'])) for r in alone
    ]
    assert [len(ruled(r.log)) for r in ran] == [2, 1]
    assert [ruled(r.log) for r in ran] == [ruled(r.log) for r in alone]


# Held runs: the rules of the panel, from a script


@dataclasses.dataclass
class Holding:
    """The runs of the rules that the test lets go on rule by rule: with the
    package each report waits at a gate; the executable waits before the
    line of each rule, and before it ends. The writing of the errors
    waits."""

    engine: Engine
    gates: Gates
    writing: Hold
    folder: Path
    run: int = 0

    @property
    def package(self) -> bool:
        return self.engine is Engine.PACKAGE

    def let(self, count: int) -> None:
        if self.package:
            self.gates[self.run].let(count)
        else:
            let_lines(self.folder, count)

    def let_all(self) -> None:
        if self.package:
            self.gates[self.run].let_all()
        else:
            let_all_lines(self.folder)
            let_end(self.folder)
        self.writing.go.set()

    def complete(self, name: str, parameters: dict[str, t.Any]) -> Ran:
        """Runs to the end, and holds the next run."""
        self.let_all()
        ran = run_algorithm(name, parameters)
        self.run += 1
        self.writing.go.clear()
        self.writing.reached.clear()
        if not self.package:
            close_gates(self.folder)
        return ran

    def held_at(self, done: int, total: int) -> bool:
        """Whether the package waits to give the report after the first
        `done`, or to write the errors. The executable waits once it has
        said its line, which the log shows."""
        if not self.package:
            return True
        if done == total:
            return self.writing.reached.is_set()
        return self.gates[self.run].reached > done

    def cancel(self, started: Started) -> None:
        started.feedback.cancel()
        self.writing.go.set()

    def release(self) -> None:
        for gate in self.gates.made:
            gate.let_all()
        self.writing.armed = False
        self.writing.go.set()
        let_all_lines(self.folder)
        let_end(self.folder)
        opened = self.gates.made[: self.gates.started]
        wait_for(lambda: all(gate.closed for gate in opened), 30)


@pytest.fixture
def holding(
    run_engine: Engine,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
) -> c.Iterator[Holding]:
    folder = tmp_path / 'gates'
    gates = Gates()
    if run_engine is Engine.PACKAGE:
        gates = hold_reports(monkeypatch)
        folder.mkdir()
    else:
        tool = tmp_path / 'tool'
        tool.mkdir()
        configure(replaying_executable(tool, folder, tmp_path / 'started'))
    held = Holding(run_engine, gates, hold_writing(monkeypatch), folder)
    yield held
    held.release()


def four_rules(tmp_path: Path) -> tuple[dict[str, t.Any], list[Case]]:
    lines, polygons = roads(), parcels()
    cases: list[Case] = [
        ('line/must-not-have-dangles', lines, None),
        ('line/must-not-overlap', lines, None),
        ('line/must-not-intersect', lines, None),
        ('polygon/must-not-overlap', polygons, None),
    ]
    parameters = {
        'RULES': json.dumps(
            [{'rule': name, 'layer': layer.id()} for name, layer, _ in cases]
        ),
        'OUTPUT': str(tmp_path / 'errors.gpkg'),
    }
    return parameters, cases


def reference_of(
    holding: Holding,
    parameters: dict[str, t.Any],
    cases: list[Case],
    tmp_path: Path,
) -> tuple[Ran, list[int], list[str]]:
    """A whole run, its counts and the lines of its rules."""
    ran = holding.complete(
        'check_rules', {**parameters, 'OUTPUT': str(tmp_path / 'all.gpkg')}
    )
    counts = json.loads(ran.results[COUNTS])
    lines = [rule_line(case, n) for case, n in zip(cases, counts, strict=True)]
    assert ruled(ran.log) == lines
    return ran, counts, lines


def test_the_log_tells_of_each_rule_as_soon_as_it_is_done(
    holding: Holding, tmp_path: Path
):
    parameters, cases = four_rules(tmp_path)
    _, _, lines = reference_of(holding, parameters, cases, tmp_path)
    started = start_algorithm('check_rules', parameters)
    for done in range(1, len(cases) + 1):
        holding.let(done)
        wait_for(
            lambda done=done: ruled(started.feedback.textLog()) == lines[:done]
        )
        wait_for(lambda done=done: holding.held_at(done, len(cases)))
        # The next rule is not told before it is let through
        assert ruled(started.feedback.textLog()) == lines[:done]
    holding.let_all()
    ran = started.ran()
    assert ran.ok
    assert ruled(ran.log) == lines


@pytest.mark.parametrize('done', range(5))
def test_a_canceled_run_of_the_rules_returns_the_rules_that_are_done(
    holding: Holding, tmp_path: Path, done: int
):
    """The task fails, as a canceled task does, so that Processing loads
    nothing. It returns the errors of the rules that are done, numbered as a
    whole run numbers them."""
    parameters, cases = four_rules(tmp_path)
    reference, counts, lines = reference_of(
        holding, parameters, cases, tmp_path
    )
    started = start_algorithm('check_rules', parameters)
    holding.let(done)
    wait_for(lambda: ruled(started.feedback.textLog()) == lines[:done])
    wait_for(lambda: holding.held_at(done, len(cases)))
    holding.cancel(started)
    ran = started.ran()
    assert not ran.ok
    assert ruled(ran.log) == lines[:done]
    if done == 0:
        assert ran.results == {}
        return
    assert json.loads(ran.results[COUNTS]) == counts[:done]
    assert ran.results[COUNT] == sum(counts[:done])
    found = sum(counts[:done])
    whole = file_rows(Path(reference.results['OUTPUT']))
    assert file_rows(Path(ran.results['OUTPUT'])) == {
        family: kept
        for family, rows in whole.items()
        if (kept := [row for row in rows if t.cast(int, row[1]) < found])
    }
    if done < len(cases):
        assert f'Canceled: {done} of {len(cases)} rules checked' in ran.log


def test_a_canceled_rule_of_the_toolbox_ends_as_before(holding: Holding):
    """Canceled before its rule is done, it fails with nothing."""
    made = line_rule()
    started = start_algorithm(rule_of(made[0]).slug, parameters_of(made))
    if holding.package:
        wait_for(lambda: holding.held_at(0, 1))
    holding.cancel(started)
    ran = started.ran()
    assert not ran.ok
    assert ran.results == {}
    assert ruled(ran.log) == []
