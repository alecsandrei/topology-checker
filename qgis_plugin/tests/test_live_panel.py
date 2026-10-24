"""The panel during a run: the count and the errors of each rule as soon as
the rule is done, what the run does now, and what stays when the run is
canceled, fails or gives way to another. The test lets a run go on rule by
rule, so that the panel is seen between any two rules."""

from __future__ import annotations

import collections.abc as c
import dataclasses
import sys
import threading
import time
import typing as t
from pathlib import Path

import pytest
from qgis.core import (
    QgsApplication,
    QgsFeature,
    QgsGeometry,
    QgsProject,
    QgsRectangle,
    QgsVariantUtils,
    QgsVectorLayer,
)
from qgis.PyQt.QtCore import QModelIndex, Qt
from qgis.PyQt.QtGui import QStandardItem

from topology_rules.core import binary, checks, engine, package
from topology_rules.core.engine import Engine
from topology_rules.core.progress import Checked, Stage
from topology_rules.exceptions import ExecutableError
from topology_rules.panel import controller, messages
from topology_rules.panel.controller import canceled_text, rule_label
from topology_rules.panel.dock import Dock
from topology_rules.panel.marks import ENTRY
from topology_rules.panel.results import seen
from topology_rules.panel.rows import Row
from topology_rules.panel.tree import ROLE, group_key
from topology_rules.utils import not_none

from .helpers import (
    add,
    click_run,
    error_layers,
    line_layer,
    message_text,
    polygon_layer,
    process_alive,
    rule_items,
    wait_for,
)
from .live import (
    Gates,
    Hold,
    close_gates,
    hold_reading,
    hold_reports,
    hold_writing,
    let_all_lines,
    let_end,
    let_lines,
    replaying_executable,
    run_validate_untold,
    validate_untold,
)
from .test_engines_agree import BOWTIE, CHAIN, SCATTERED, SHIFTED, SQUARE

DANGLES = 'line/must-not-have-dangles'
LINE_OVERLAPS = 'line/must-not-overlap'
INTERSECTS = 'line/must-not-intersect'
OVERLAPS = 'polygon/must-not-overlap'
CROSSING = 'LINESTRING(0 -5, 5 5)'
# The rules of most runs here: the second finds nothing
RULES = 4
ERRORS_TAB = controller.ERRORS_TAB
# How long a step that the panel takes may hold up the interface, as for the
# runs of the tests of performance
MAX_STALL = 3.0


def no_executable() -> Path:
    raise ExecutableError('The package runs the checks, not the executable')


@dataclasses.dataclass
class Held:
    """The runs of an engine, which the test lets go on rule by rule: with
    the package each report waits at a gate, and the executable waits
    before each line of progress, and before it ends."""

    engine: Engine
    gates: Gates
    writing: Hold
    folder: Path
    started: Path
    run: int = 0

    @property
    def package(self) -> bool:
        return self.engine is Engine.PACKAGE

    def let(self, count: int) -> None:
        """Lets the run tell of its first `count` rules."""
        if self.package:
            self.gates[self.run].let(count)
        else:
            let_lines(self.folder, count)

    def let_all(self) -> None:
        """Lets the run go to its end."""
        if self.package:
            self.gates[self.run].let_all()
            self.writing.go.set()
        else:
            let_all_lines(self.folder)
            let_end(self.folder)

    def next_run(self) -> None:
        """Holds the next run from the start."""
        self.run += 1
        self.writing.go.clear()
        self.writing.reached.clear()
        if not self.package:
            close_gates(self.folder)
            self.started.unlink(missing_ok=True)

    def start(self, dock: Dock) -> controller.Run:
        dock.run_button.click()
        assert dock.run is not None
        return dock.run

    def complete(self, dock: Dock) -> Seen:
        """Runs to the end, and holds the next run."""
        self.let_all()
        click_run(dock)
        now = seen_now(dock)
        self.next_run()
        return now

    def cancel(self, dock: Dock) -> None:
        """Cancels the run, which writes the errors of the rules that are
        done."""
        dock.cancel()
        self.writing.go.set()

    def pid(self) -> int:
        """The process number of the tool that runs."""
        wait_for(self.started.exists)
        return int(self.started.read_text())

    def at_end(self) -> bool:
        """Whether the run is done with its rules and writes the errors."""
        return self.writing.reached.is_set()

    def release(self) -> None:
        """Lets every run go, and waits for the streams of the package to
        close."""
        for gate in self.gates.made:
            gate.let_all()
        self.writing.armed = False
        self.writing.go.set()
        let_all_lines(self.folder)
        let_end(self.folder)
        started = self.gates.made[: self.gates.started]
        wait_for(lambda: all(gate.closed for gate in started), 30)


@pytest.fixture(
    params=[Engine.EXECUTABLE, Engine.PACKAGE],
    ids=[Engine.EXECUTABLE.value, Engine.PACKAGE.value],
)
def held(
    request: pytest.FixtureRequest,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    executable: str,
) -> Held:
    chosen = t.cast(Engine, request.param)
    folder, started = tmp_path / 'gates', tmp_path / 'started'
    gates = Gates()
    if chosen is Engine.PACKAGE:
        request.getfixturevalue('package')
        gates = hold_reports(monkeypatch)
        monkeypatch.setattr(binary, 'find_executable', no_executable)
    else:
        tool = tmp_path / 'tool'
        tool.mkdir()
        configure(replaying_executable(tool, folder, started))
    folder.mkdir(exist_ok=True)
    engine.choose(chosen)
    return Held(chosen, gates, hold_writing(monkeypatch), folder, started)


# For a test of the held runs of the executable only
EXECUTABLE_ONLY = pytest.mark.parametrize(
    'held', [Engine.EXECUTABLE], ids=[Engine.EXECUTABLE.value], indirect=True
)


@pytest.fixture
def held_package(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    executable: str,
    package: object,
) -> Held:
    """The runs of the package only, whose rules come with their errors."""
    gates = hold_reports(monkeypatch)
    monkeypatch.setattr(binary, 'find_executable', no_executable)
    engine.choose(Engine.PACKAGE)
    folder = tmp_path / 'gates'
    folder.mkdir()
    return Held(
        Engine.PACKAGE,
        gates,
        hold_writing(monkeypatch),
        folder,
        tmp_path / 'started',
    )


def ready(dock: Dock) -> Dock:
    wait_for(lambda: dock.probing is None and dock.available)
    return dock


@pytest.fixture
def panel(held: Held, dock: Dock) -> c.Iterator[Dock]:
    """A panel of the held engine, which lets the runs go when the test is
    done, whatever it left them at."""
    yield ready(dock)
    held.release()


@pytest.fixture
def package_panel(held_package: Held, dock: Dock) -> c.Iterator[Dock]:
    yield ready(dock)
    held_package.release()


# Layers and rules


def project_layer(layer: QgsVectorLayer) -> QgsVectorLayer:
    QgsProject.instance().addMapLayer(layer)
    return layer


def roads() -> QgsVectorLayer:
    return project_layer(line_layer(*CHAIN, CROSSING, name='roads'))


def parcels() -> QgsVectorLayer:
    return project_layer(polygon_layer(SQUARE, SHIFTED, name='parcels'))


def add_rules(dock: Dock) -> tuple[QgsVectorLayer, QgsVectorLayer]:
    """Four rules on two layers, the second of which finds nothing."""
    lines, polygons = roads(), parcels()
    for name in (DANGLES, LINE_OVERLAPS, INTERSECTS):
        add(dock, name, lines)
    add(dock, OVERLAPS, polygons)
    assert len(dock.entries) == RULES
    return lines, polygons


# What the panel shows


Tree = list[tuple[str, str, list[tuple[str, list[tuple[str, bool]]]]]]


def tree_of(dock: Dock) -> Tree:
    """Each group of the tree with its key and text, each kind of it with
    its text, and each error with its text and whether it is grey."""
    model = dock.results.model
    found: Tree = []
    for row in range(model.rowCount()):
        group = not_none(model.item(row))
        kinds: list[tuple[str, list[tuple[str, bool]]]] = []
        for k in range(group.rowCount()):
            kind = not_none(group.child(k))
            errors = [
                (
                    not_none(kind.child(i)).text(),
                    not_none(kind.child(i)).foreground().color()
                    == Qt.GlobalColor.gray,
                )
                for i in range(kind.rowCount())
            ]
            kinds.append((kind.text(), errors))
        found.append((group_key(group), group.text(), kinds))
    return found


def counts_of(dock: Dock) -> list[str]:
    """The count that the list shows for each rule, in the order of the
    rules of the run."""
    texts = {item.data(0, ENTRY): item.text(1) for item in rule_items(dock)}
    return [texts[index] for index in range(len(dock.entries))]


# The name of the layer, the number of the error, its columns and geometry
MapRow = tuple[str, int, tuple[object, ...], str]


def map_rows() -> list[MapRow]:
    """The errors on the map: each feature of each layer of errors. Errors
    are numbered rule after rule; features the rules skipped have no number,
    -1 here."""
    rows: list[MapRow] = []
    for layer in error_layers():
        for feature in not_none(layer.dataProvider()).getFeatures():
            number = feature['violation_id']
            rows.append(
                (
                    layer.name(),
                    -1 if QgsVariantUtils.isNull(number) else int(number),
                    tuple(feature.attributes()),
                    feature.geometry().asWkt(6),
                )
            )
    return sorted(rows, key=repr)


@dataclasses.dataclass
class Seen:
    counts: list[str]
    tree: Tree
    tab: str
    map: list[MapRow]

    def first(self, count: int) -> Seen:
        """What a run that was done with the first rules showed of them."""
        keys = {f'e{index}' for index in range(count)}
        found = sum(int(number) for number in self.counts[:count])
        errors = [row for row in self.map if 0 <= row[1] < found]
        return Seen(
            self.counts[:count] + [''] * (len(self.counts) - count),
            [group for group in self.tree if group[0] in keys],
            errors_tab(found),
            errors,
        )


def seen_now(dock: Dock) -> Seen:
    return Seen(
        counts_of(dock),
        tree_of(dock),
        dock.tabs.tabText(ERRORS_TAB),
        map_rows(),
    )


def errors_tab(count: int) -> str:
    return f'Errors ({count})' if count else 'Errors'


def without_map(view: Seen) -> Seen:
    return dataclasses.replace(view, map=[])


def checking(dock: Dock, index: int, total: int = RULES) -> str:
    """What the panel says while the run checks the rule at the index."""
    label = rule_label(dock.entries[index])
    if total == 1:
        return f'Checking {label}…'
    return f'Checking {label} ({index + 1} of {total})…'


def stage_after(dock: Dock, held: Held, done: int, total: int = RULES) -> str:
    """What the panel says once a number of rules are done."""
    if done == total:
        return 'Writing errors…'
    if not held.package and done == 0:
        return 'Checking 1 rule…' if total == 1 else f'Checking {total} rules…'
    return checking(dock, done, total)


def eventually(get: c.Callable[[], object], expected: object) -> None:
    """Runs the event loop until what is got is as expected, which the run
    tells in an event of its own, then compares them."""
    deadline = time.monotonic() + 10
    while get() != expected and time.monotonic() < deadline:
        QgsApplication.processEvents()
        time.sleep(0.01)
    assert get() == expected


def at(
    dock: Dock, held: Held, run: controller.Run, done: int, total: int = RULES
) -> None:
    """Waits for the panel to show that the run did the first rules, and
    is at the next, or writes the errors."""
    wait_for(lambda: len(run.done) >= done)
    assert sorted(run.done) == list(range(done))
    eventually(dock.stage.full, stage_after(dock, held, done, total))
    if held.package and done == total:
        wait_for(held.at_end)


def bar_range(dock: Dock) -> tuple[int, int, int]:
    bar = dock.progress
    return bar.minimum(), bar.maximum(), bar.value()


def last_message(bar: t.Any) -> str:
    return str(bar.calls[-1][0][1]) if bar.calls else ''


# During a run


def test_each_rule_shows_its_count_and_errors_as_soon_as_it_is_done(
    held: Held, panel: Dock
):
    dock = panel
    add_rules(dock)
    reference = held.complete(dock)
    assert reference.counts[1] == '0'
    assert all(int(count) for i, count in enumerate(reference.counts) if i != 1)
    dock.tabs.setCurrentIndex(0)
    run = held.start(dock)
    for done in range(RULES + 1):
        at(dock, held, run, done)
        expected = reference.first(done)
        now = seen_now(dock)
        assert now.counts == expected.counts
        assert now.tab == expected.tab
        # No layer of errors until the run ends
        assert now.map == []
        if held.package:
            assert now.tree == expected.tree
        else:
            # The executable gives the errors with its file only
            assert now.tree == []
            placeholder = dock.results.placeholder.text()
            assert placeholder == (
                'The errors show when the run ends.'
                if done
                else 'No errors yet.'
            )
        # The run does not take the user to the errors
        assert dock.tabs.currentIndex() == 0
        assert dock.run_button.text() == 'Cancel'
        held.let(done + 1)
    held.let_all()
    wait_for(lambda: dock.task is None)
    assert seen_now(dock) == reference
    assert dock.tabs.currentIndex() == ERRORS_TAB


def test_the_progress_goes_with_the_rules_after_the_reading(
    held: Held, panel: Dock, monkeypatch: pytest.MonkeyPatch
):
    dock = panel
    add_rules(dock)
    reading = hold_reading(monkeypatch)
    run = held.start(dock)
    wait_for(reading.reached.is_set)
    assert dock.stage.full() == 'Reading layers…'
    assert not dock.stage.isHidden()
    # Busy: how long the layers take to read is not known
    assert bar_range(dock) == (0, 0, bar_range(dock)[2])
    assert counts_of(dock) == [''] * RULES
    assert dock.tabs.tabText(ERRORS_TAB) == 'Errors'
    assert tree_of(dock) == []
    assert dock.results.placeholder.text() == 'No errors yet.'
    reading.go.set()
    for done in range(RULES + 1):
        at(dock, held, run, done)
        if held.package or done:
            eventually(lambda: bar_range(dock), (0, RULES, done))
        else:
            # The executable says which rule it checks once one is done
            assert bar_range(dock)[:2] == (0, 0)
        held.let(done + 1)
    held.let_all()
    wait_for(lambda: dock.task is None)
    assert dock.stage.isHidden()
    assert dock.progress.isHidden()


def test_a_run_of_one_rule_says_the_rule_without_a_number(
    held: Held, panel: Dock
):
    dock = panel
    add(dock, DANGLES, roads())
    run = held.start(dock)
    wait_for(lambda: dock.stage.full() != 'Reading layers…')
    expected = checking(dock, 0, 1) if held.package else 'Checking 1 rule…'
    eventually(dock.stage.full, expected)
    held.let(1)
    wait_for(lambda: len(run.done) == 1)
    eventually(dock.stage.full, 'Writing errors…')
    held.let_all()
    wait_for(lambda: dock.task is None)
    assert counts_of(dock) != ['']


def test_the_stage_fits_a_narrow_panel(held: Held, panel: Dock):
    """The name of the rule is cut short, and the number of the rule and
    the whole text in the tooltip stay."""
    dock = panel
    long_name = 'roads of the county and of the towns around it'
    lines = project_layer(line_layer(*CHAIN, name=long_name))
    add(dock, DANGLES, lines)
    add(dock, INTERSECTS, lines)
    dock.resize(180, 600)
    dock.show()
    run = held.start(dock)
    held.let(1)
    at(dock, held, run, 1, total=2)
    QgsApplication.processEvents()
    label = dock.stage
    full = checking(dock, 1, 2)
    assert label.full() == full
    assert label.toolTip() == full
    assert label.text() != full
    assert label.text().startswith('Checking ')
    assert label.text().endswith(' (2 of 2)…')
    room = label.contentsRect().width()
    assert label.fontMetrics().horizontalAdvance(label.text()) <= room
    # Wide again, the whole text shows
    dock.resize(1200, 600)
    QgsApplication.processEvents()
    assert label.text() == full
    held.let_all()
    wait_for(lambda: dock.task is None)


# Cancel


@pytest.mark.parametrize('done', range(RULES + 1))
def test_canceling_keeps_the_rules_that_are_done(
    held: Held, panel: Dock, bar: t.Any, done: int
):
    dock = panel
    add_rules(dock)
    reference = held.complete(dock)
    dock.tabs.setCurrentIndex(0)
    run = held.start(dock)
    held.let(done)
    at(dock, held, run, done)
    pid = None if held.package else held.pid()
    bar.calls.clear()
    held.cancel(dock)
    wait_for(lambda: dock.task is None)
    expected = reference.first(done)
    now = seen_now(dock)
    assert now.counts == expected.counts
    found = sum(int(count) for count in reference.counts[:done])
    if held.package:
        # As a run of those rules shows them, on the map too
        assert now == expected
    else:
        # The executable is stopped before it writes the errors
        assert pid is not None
        assert not process_alive(pid)
        assert now == Seen(expected.counts, [], 'Errors', [])
    assert last_message(bar) == canceled_text(done, RULES, found)
    assert dock.tabs.currentIndex() == 0
    assert dock.stage.isHidden()
    assert dock.run_button.text() == f'Run {RULES} rules'


def test_canceling_while_a_rule_gives_its_errors_drops_that_rule(
    held_package: Held,
    package_panel: Dock,
    bar: t.Any,
    monkeypatch: pytest.MonkeyPatch,
):
    """The rule whose errors were being read is not done."""
    dock = package_panel
    held = held_package
    lines, polygons = roads(), parcels()
    add(dock, INTERSECTS, lines)
    add(dock, DANGLES, lines)
    add(dock, OVERLAPS, polygons)
    total = len(dock.entries)
    reference = held.complete(dock)
    run = held.start(dock)
    # The run is canceled at the first error of the dangles, which are more
    counts = [int(count) for count in reference.counts]
    rule = 1
    assert counts[rule] > 1
    calls: list[int] = []
    real = package.attributes
    first = sum(counts[:rule])

    def attributes(index: int, *args: t.Any) -> list[object]:
        calls.append(index)
        if index == first:
            run.feedback.cancel()
        return real(index, *args)

    monkeypatch.setattr(package, 'attributes', attributes)
    held.let_all()
    wait_for(lambda: dock.task is None)
    assert first in calls
    assert seen_now(dock) == reference.first(rule)
    assert last_message(bar) == canceled_text(rule, total, first)


def test_canceling_stops_the_rule_that_the_package_checks(
    held_package: Held, package_panel: Dock
):
    """The package stops checking at once, not when the rule is done."""
    import topology_checker as tc

    dock = package_panel
    held = held_package
    add_rules(dock)
    run = held.start(dock)
    held.let(1)
    at(dock, held, run, 1)
    held.cancel(dock)
    wait_for(lambda: dock.task is None)
    # The thread of the package that checks the rules is gone, while the
    # thread that waits for the reports may still be held by the test
    assert tc._native.live_workers(10.0) == 0


def test_canceling_while_the_layers_are_read(
    held: Held, panel: Dock, bar: t.Any, monkeypatch: pytest.MonkeyPatch
):
    dock = panel
    add_rules(dock)
    reading = hold_reading(monkeypatch)
    held.start(dock)
    wait_for(reading.reached.is_set)
    bar.calls.clear()
    held.cancel(dock)
    reading.go.set()
    wait_for(lambda: dock.task is None)
    assert seen_now(dock) == Seen([''] * RULES, [], 'Errors', [])
    assert last_message(bar) == 'Canceled.'


# Failures


@pytest.mark.parametrize('after', range(RULES))
def test_a_package_that_fails_keeps_what_it_showed(
    held_package: Held, package_panel: Dock, bar: t.Any, after: int
):
    """The rules that are done keep their counts and errors, with no layer
    of errors, and the message says what went wrong."""
    dock = package_panel
    held = held_package
    add_rules(dock)
    reference = held.complete(dock)
    gate = held.gates[held.run]
    gate.fail_at = after
    gate.failure = 'the package ran out of memory'
    bar.clear_messages()
    bar.clearWidgets()
    run = held.start(dock)
    held.let_all()
    wait_for(lambda: dock.task is None)
    assert sorted(run.done) == list(range(after))
    expected = reference.first(after)
    assert seen_now(dock) == dataclasses.replace(expected, map=[])
    assert 'the package ran out of memory' in message_text(bar)
    assert dock.stage.isHidden()


def crashing_tool(
    held: Held,
    dock: Dock,
    configure: c.Callable[[str | Path], None],
    **how: int,
) -> None:
    """Points the panel to a tool that replays the real one and stops in
    the middle as said."""
    tool = held.folder.parent / 'crashing'
    tool.mkdir()
    configure(replaying_executable(tool, held.folder, held.started, **how))
    dock.refresh()
    ready(dock)


@EXECUTABLE_ONLY
@pytest.mark.parametrize('after', range(RULES))
def test_a_tool_that_crashes_keeps_the_counts_of_the_rules_that_are_done(
    held: Held,
    panel: Dock,
    bar: t.Any,
    configure: c.Callable[[str | Path], None],
    after: int,
):
    dock = panel
    add_rules(dock)
    reference = held.complete(dock)
    crashing_tool(held, dock, configure, crash_after=after)
    bar.clear_messages()
    bar.clearWidgets()
    held.start(dock)
    held.let_all()
    wait_for(lambda: dock.task is None)
    expected = reference.first(after)
    assert seen_now(dock) == Seen(expected.counts, [], 'Errors', [])
    assert 'the tool broke' in message_text(bar)


@pytest.mark.skipif(sys.platform == 'win32', reason='a signal of POSIX')
@EXECUTABLE_ONLY
@pytest.mark.parametrize('after', [0, 2])
def test_a_tool_that_is_killed_keeps_the_counts_of_the_rules_that_are_done(
    held: Held,
    panel: Dock,
    bar: t.Any,
    configure: c.Callable[[str | Path], None],
    after: int,
):
    dock = panel
    add_rules(dock)
    reference = held.complete(dock)
    crashing_tool(held, dock, configure, killed_after=after)
    bar.clear_messages()
    bar.clearWidgets()
    held.start(dock)
    held.let_all()
    wait_for(lambda: dock.task is None)
    expected = reference.first(after)
    assert seen_now(dock) == Seen(expected.counts, [], 'Errors', [])
    # As before: the code the tool ended with, the number of the signal
    assert message_text(bar).strip() == 'Exited with code -9'


# Which run the panel waits for


def stale_events(run: controller.Run) -> None:
    """What an old run could still tell: a rule that is done, and the
    stage after it."""
    total = len(run.entries)
    done = Checked(total - 1, total, 99, None, {})
    run.feedback.rule_done.emit((done, None))
    run.feedback.stage_changed.emit((Stage.CHECKING, total - 1, total))
    run.feedback.stage_changed.emit((Stage.READING, None, 0))
    for _ in range(5):
        QgsApplication.processEvents()


@pytest.mark.parametrize('ended', ['canceled', 'finished'])
def test_a_new_run_gets_nothing_of_the_last_one(
    held: Held, panel: Dock, ended: str
):
    dock = panel
    add_rules(dock)
    reference = held.complete(dock)
    old = held.start(dock)
    held.let(1)
    at(dock, held, old, 1)
    if ended == 'canceled':
        held.cancel(dock)
    else:
        held.let_all()
    wait_for(lambda: dock.task is None)
    held.next_run()
    new = held.start(dock)
    held.let(1)
    at(dock, held, new, 1)
    before = (seen_now(dock), dock.stage.full(), bar_range(dock))
    stale_events(old)
    assert (seen_now(dock), dock.stage.full(), bar_range(dock)) == before
    held.let_all()
    wait_for(lambda: dock.task is None)
    assert seen_now(dock) == reference


def test_what_a_run_tells_after_its_end_changes_nothing(
    held: Held, panel: Dock
):
    dock = panel
    add_rules(dock)
    run = held.start(dock)
    held.let_all()
    wait_for(lambda: dock.task is None)
    after = seen_now(dock)
    stale_events(run)
    assert seen_now(dock) == after
    assert dock.stage.isHidden()
    assert dock.progress.isHidden()


def test_clearing_the_project_during_a_run_drops_it(
    held: Held, panel: Dock, bar: t.Any
):
    dock = panel
    add_rules(dock)
    run = held.start(dock)
    held.let(1)
    at(dock, held, run, 1)
    bar.calls.clear()
    QgsProject.instance().clear()
    assert dock.task is None
    held.let_all()
    wait_for(lambda: run not in controller.LIVE)
    for _ in range(5):
        QgsApplication.processEvents()
    assert error_layers() == []
    assert dock.entries == []
    assert tree_of(dock) == []
    assert dock.tabs.tabText(ERRORS_TAB) == 'Errors'
    assert dock.stage.isHidden()
    assert bar.calls == []


def test_unloading_the_plugin_during_a_run(
    held: Held,
    qgis_iface: t.Any,
    qgis_processing: None,
    bar: t.Any,
):
    from topology_rules.plugin import TopologyRules

    plugin = TopologyRules(qgis_iface)
    plugin.initGui()
    dock = plugin.dock
    dock.refresh()
    ready(dock)
    add_rules(dock)
    run = held.start(dock)
    held.let(1)
    wait_for(lambda: len(run.done) == 1)
    bar.calls.clear()
    del dock
    plugin.unload()
    held.release()
    wait_for(lambda: run not in controller.LIVE)
    for _ in range(5):
        QgsApplication.processEvents()
    assert error_layers() == []
    assert bar.calls == []


def test_closing_and_opening_the_panel_during_a_run(held: Held, panel: Dock):
    dock = panel
    add_rules(dock)
    reference = held.complete(dock)
    dock.show()
    run = held.start(dock)
    held.let(1)
    at(dock, held, run, 1)
    dock.close()
    QgsApplication.processEvents()
    held.let(2)
    at(dock, held, run, 2)
    dock.show()
    QgsApplication.processEvents()
    assert dock.task is not None
    assert not dock.stage.isHidden()
    assert dock.stage.full() == stage_after(dock, held, 2)
    now = seen_now(dock)
    expected = reference.first(2)
    assert now.counts == expected.counts
    if held.package:
        assert now.tree == expected.tree
    held.let_all()
    wait_for(lambda: dock.task is None)
    assert seen_now(dock) == reference


@pytest.mark.parametrize('which', ['roads', 'parcels'])
def test_removing_a_layer_of_the_rules_during_a_run(
    held: Held, panel: Dock, bar: t.Any, which: str
):
    """The run goes on with what it read. Its errors are shown, and the
    rules of the layer that is gone are marked as missing."""
    dock = panel
    lines, polygons = add_rules(dock)
    reference = held.complete(dock)
    run = held.start(dock)
    held.let(1)
    at(dock, held, run, 1)
    gone = lines if which == 'roads' else polygons
    QgsProject.instance().removeMapLayer(gone.id())
    QgsApplication.processEvents()
    held.let_all()
    wait_for(lambda: dock.task is None)
    now = seen_now(dock)
    assert now.counts == reference.counts
    assert [group[0] for group in now.tree] == [
        group[0] for group in reference.tree
    ]
    assert len(now.map) == len(reference.map)
    missing = [entry.missing for entry in dock.entries]
    assert missing == [which == 'roads'] * 3 + [which == 'parcels']
    assert 'errors in 4 rules' in last_message(bar)


# The errors of a rule that is done, during the run


def first_error(dock: Dock, key: str = 'e0') -> QModelIndex:
    model = dock.results.model
    for row in range(model.rowCount()):
        group = not_none(model.item(row))
        if group_key(group) == key:
            return not_none(not_none(group.child(0)).child(0)).index()
    raise AssertionError(f'no group {key}')


def test_an_error_of_a_rule_that_is_done_works_during_the_run(
    held_package: Held, package_panel: Dock, qgis_iface: t.Any
):
    dock = package_panel
    held = held_package
    lines, _ = add_rules(dock)
    reference = held.complete(dock)
    canvas = not_none(qgis_iface.mapCanvas())
    canvas.setDestinationCrs(lines.crs())
    # Away from the errors
    canvas.setExtent(QgsRectangle(100000, 100000, 101000, 101000))
    run = held.start(dock)
    held.let(1)
    at(dock, held, run, 1)
    view = dock.results_view
    index = first_error(dock)
    view.expand(index.parent().parent())
    view.expand(index.parent())
    view.setCurrentIndex(index)
    row = t.cast(Row, index.data(ROLE))
    assert row.error_layer == ''
    # Zoomed to and marked, from its own geometry
    focus = dock.results.focus
    assert focus.highlight is not None
    assert focus.highlight.isVisible()
    assert canvas.extent().contains(
        not_none(row.geometry).boundingBox().center()
    )
    assert dock.results.select_button.isEnabled()
    dock.results.select_button.click()
    assert lines.selectedFeatureIds() == [row.feature]
    # Ignored, it is grey once shown, and counts no more
    dock.results.ignore_action.trigger()
    first = int(reference.counts[0])
    assert counts_of(dock)[0] == str(first - 1)
    assert dock.tabs.tabText(ERRORS_TAB) == errors_tab(first - 1)
    assert dock.results.show_ignored.text() == 'Show ignored (1)'
    shown_errors = [
        error
        for key, _, kinds in tree_of(dock)
        if key == 'e0'
        for _, errors in kinds
        for error in errors
    ]
    assert len(shown_errors) == first - 1
    dock.results.show_ignored.setChecked(True)
    grey = [
        text
        for _, _, kinds in tree_of(dock)
        for _, errors in kinds
        for text, is_grey in errors
        if is_grey
    ]
    assert len(grey) == 1
    dock.results.show_ignored.setChecked(False)
    # A group opens and closes
    group = dock.results.model.index(0, 0)
    view.collapse(group)
    assert not view.isExpanded(group)
    view.expand(group)
    assert view.isExpanded(group)
    held.let_all()
    wait_for(lambda: dock.task is None)
    # The error stays ignored, now in its layer
    total = sum(int(count) for count in reference.counts)
    assert counts_of(dock)[0] == str(first - 1)
    assert dock.tabs.tabText(ERRORS_TAB) == errors_tab(total - 1)
    ignored = [r for r in dock.results.rows if dock.results.is_ignored(r)]
    assert len(ignored) == 1
    assert ignored[0].error_layer
    assert seen(ignored[0]) == seen(row)
    layer = not_none(QgsProject.instance().mapLayer(ignored[0].error_layer))
    assert f'NOT IN ({ignored[0].error_fid})' in layer.subsetString()


def expanded(dock: Dock) -> list[tuple[str, str | None]]:
    """The groups and kinds that are open, by key and kind."""
    view = dock.results_view
    model = dock.results.model
    found: list[tuple[str, str | None]] = []
    for row in range(model.rowCount()):
        group = not_none(model.item(row))
        if view.isExpanded(group.index()):
            found.append((group_key(group), None))
        for k in range(group.rowCount()):
            kind = not_none(group.child(k))
            if view.isExpanded(kind.index()):
                found.append((group_key(group), kind.data(ROLE)[2]))
    return found


KEPT = Qt.ItemDataRole.UserRole + 77


def test_the_end_of_a_run_keeps_the_tree_as_the_user_left_it(
    held_package: Held, package_panel: Dock
):
    """Open groups, the current error, its selection and the scroll stay,
    and the groups that were shown are not built again."""
    dock = package_panel
    held = held_package
    scattered = project_layer(line_layer(*SCATTERED, name='scattered'))
    add(dock, DANGLES, scattered)
    add(dock, INTERSECTS, scattered)
    lines = roads()
    add(dock, DANGLES, lines)
    add(dock, OVERLAPS, parcels())
    reference = held.complete(dock)
    dock.resize(320, 360)
    dock.show()
    dock.tabs.setCurrentIndex(ERRORS_TAB)
    run = held.start(dock)
    held.let(3)
    at(dock, held, run, 3)
    view = dock.results_view
    model = dock.results.model
    index = first_error(dock)
    view.expand(index.parent().parent())
    view.expand(index.parent())
    kind = index.parent()
    current = model.index(5, 0, kind)
    view.setCurrentIndex(current)
    row = t.cast(Row, current.data(ROLE))
    QgsApplication.processEvents()
    scroll = not_none(view.verticalScrollBar())
    assert scroll.maximum() > 0
    scroll.setValue(scroll.maximum() // 2)
    QgsApplication.processEvents()
    opened = expanded(dock)
    position = scroll.value()
    items: list[QStandardItem] = []
    for number in range(model.rowCount()):
        item = not_none(model.item(number))
        item.setData('kept', KEPT)
        items.append(item)
    signals: list[str] = []
    model.modelReset.connect(lambda: signals.append('reset'))
    model.rowsRemoved.connect(lambda *_: signals.append('removed'))
    selection = not_none(view.selectionModel())
    selection.selectionChanged.connect(lambda *_: signals.append('selection'))
    held.let_all()
    wait_for(lambda: dock.task is None)
    assert dock.tabs.currentIndex() == ERRORS_TAB
    assert expanded(dock) == opened
    assert scroll.value() == position
    now = dock.results.current_row()
    assert now is not None
    assert seen(now) == seen(row)
    assert now.error_layer
    assert [index.row() for index in selection.selectedIndexes()] == [5]
    assert view.currentIndex() == selection.selectedIndexes()[0]
    # The groups that were there are the same items
    for number, item in enumerate(items):
        assert model.item(number) is item
        assert item.data(KEPT) == 'kept'
    assert signals == []
    # The map shows the errors of the kind of the current error
    expression = {layer.subsetString() for layer in error_layers()}
    view.clearSelection()
    view.setCurrentIndex(current)
    assert {layer.subsetString() for layer in error_layers()} == expression
    assert tree_of(dock) == reference.tree


def test_a_rule_of_thousands_of_errors_does_not_hold_up_the_panel(
    held_package: Held, package_panel: Dock, monkeypatch: pytest.MonkeyPatch
):
    """Its errors are made ready in the thread of the run, and the panel
    only shows them."""
    dock = package_panel
    held = held_package
    spikes = 2500
    layer = QgsVectorLayer('LineString?crs=EPSG:32633', 'spikes', 'memory')
    features: list[QgsFeature] = []
    for index in range(spikes):
        feature = QgsFeature()
        feature.setGeometry(
            QgsGeometry.fromWkt(f'LINESTRING({3 * index} 0, {3 * index + 1} 0)')
        )
        features.append(feature)
    assert not_none(layer.dataProvider()).addFeatures(features)[0]
    project_layer(layer)
    add(dock, DANGLES, layer)
    add(dock, DANGLES, roads())
    made_in: list[str] = []
    real = messages.found_rows

    def found_rows(*args: t.Any) -> list[Row]:
        made_in.append(threading.current_thread().name)
        return real(*args)

    monkeypatch.setattr(messages, 'found_rows', found_rows)
    dock.resize(320, 600)
    dock.show()
    dock.tabs.setCurrentIndex(ERRORS_TAB)
    run = held.start(dock)

    def longest(until: c.Callable[[], bool]) -> float:
        most = 0.0
        deadline = time.monotonic() + 120
        while not until():
            assert time.monotonic() < deadline
            before = time.monotonic()
            QgsApplication.processEvents()
            most = max(most, time.monotonic() - before)
            time.sleep(0.01)
        return most

    held.let(1)
    mid = longest(lambda: len(run.done) == 1)
    assert counts_of(dock)[0] == str(2 * spikes)
    held.let_all()
    end = longest(lambda: dock.task is None)
    assert counts_of(dock)[0] == str(2 * spikes)
    assert threading.main_thread().name not in made_in
    assert len(made_in) == 2
    assert mid < MAX_STALL, mid
    assert end < MAX_STALL, end


# A run that ends shows what it did before


@dataclasses.dataclass
class Shown:
    """All that the panel shows at the end of a run."""

    view: Seen
    message: str
    invalid: dict[str, int]
    current: int


def shown_now(dock: Dock, bar: t.Any) -> Shown:
    return Shown(
        seen_now(dock),
        last_message(bar),
        dict(dock.rule_list.invalid),
        dock.tabs.currentIndex(),
    )


def rules_with_invalid(dock: Dock) -> None:
    polygons = project_layer(
        polygon_layer(SQUARE, SHIFTED, BOWTIE, name='parcels')
    )
    add(dock, OVERLAPS, polygons)
    add(dock, DANGLES, roads())


def rules_of_two_layers(dock: Dock) -> None:
    lines = roads()
    rivers = project_layer(line_layer('LINESTRING(5 -5, 5 5)', name='rivers'))
    add(dock, 'line/must-not-intersect-with', lines, rivers)
    add(dock, 'line/must-be-inside', lines, parcels())


def rules_without_errors(dock: Dock) -> None:
    ring = project_layer(
        line_layer('LINESTRING(0 0, 10 0, 10 10, 0 0)', name='ring')
    )
    add(dock, DANGLES, ring)
    add(dock, INTERSECTS, ring)


def one_rule(dock: Dock) -> None:
    add(dock, DANGLES, roads())


def selected_only(dock: Dock) -> None:
    lines, _ = add_rules(dock)
    lines.selectByIds([1, 4])
    dock.selected_only.setChecked(True)


RUNS: dict[str, c.Callable[[Dock], object]] = {
    'four-rules': add_rules,
    'two-layers': rules_of_two_layers,
    'invalid': rules_with_invalid,
    'no-errors': rules_without_errors,
    'one-rule': one_rule,
    'selected-only': selected_only,
}


@pytest.mark.parametrize('rules', list(RUNS))
def test_a_run_ends_as_runs_did_before_they_told_of_each_rule(
    run_engine: Engine,
    dock: Dock,
    bar: t.Any,
    rules: str,
    monkeypatch: pytest.MonkeyPatch,
):
    ready(dock)
    RUNS[rules](dock)
    bar.calls.clear()
    click_run(dock)
    told = shown_now(dock, bar)
    with monkeypatch.context() as patched:
        patched.setattr(package, 'validate', validate_untold)
        patched.setattr(checks, 'run_validate', run_validate_untold)
        bar.calls.clear()
        dock.tabs.setCurrentIndex(0)
        click_run(dock)
        before = shown_now(dock, bar)
    assert told == before
