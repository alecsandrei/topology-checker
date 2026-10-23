"""Helpers of the tests, grouped by what they work on.

Fake executables stand in for the tool, layers are small files or memory
layers, `run_algorithm` runs a Processing algorithm as the dock does, and the
last groups read the widgets of the dock the way a user sees them.
"""

from __future__ import annotations

import dataclasses
import faulthandler
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import time
import typing as t
from pathlib import Path

import pytest
from qgis.core import (
    Qgis,
    QgsApplication,
    QgsFeature,
    QgsGeometry,
    QgsProcessingAlgRunnerTask,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
    QgsSettings,
    QgsVectorFileWriter,
    QgsVectorLayer,
)
from qgis.PyQt.QtCore import QItemSelectionModel, QModelIndex, Qt
from qgis.PyQt.QtWidgets import QApplication, QLabel, QTreeWidgetItem

from topology_rules.core import engine
from topology_rules.core.engine import Engine
from topology_rules.enums import PROVIDER_ID

if t.TYPE_CHECKING:
    from topology_rules.panel.dock import Dock
RULES = [
    {
        'name': 'line/must-not-have-dangles',
        'description': 'Report part endpoints with one incident use.',
        'arcgis': None,
        'qgis': None,
        'layer': 'line',
        'other': None,
    },
    {
        'name': 'point/must-be-covered-by-lines',
        'description': 'Points must lie on a line.',
        'arcgis': None,
        'qgis': None,
        'layer': 'point',
        'other': 'line',
    },
]


# Fake executables


def write_tool(path: Path, source: str) -> Path:
    """Writes a program of Python source and returns the path to point the
    plugin to. Where a shebang runs it, that is the file itself. Windows
    ignores the shebang, so there a `.cmd` beside the script starts it with
    this Python, and the launcher is what is returned. It starts without the
    site packages and the environment (-S, -I), as the scripts only use the
    standard library and a start with them is quicker, which matters when a
    run starts the tool many times."""
    path.write_text(source)
    if sys.platform != 'win32':
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
        return path
    script = path.with_name(f'{path.name}.py')
    path.replace(script)
    launcher = path.with_name(f'{path.name}.cmd')
    launcher.write_bytes(
        f'@"{sys.executable}" -I -S "%~dp0{script.name}" %*\r\n'.encode()
    )
    return launcher


def hand_over(real: str) -> str:
    """The lines of a fake tool that make it the real tool, with the same
    arguments. On POSIX the program is replaced by the real tool. Windows has
    no such call, so the fake waits for the real tool and ends as it does."""
    if sys.platform == 'win32':
        return (
            'import subprocess\n'
            f'sys.exit(subprocess.call([{real!r}, *sys.argv[1:]]))\n'
        )
    return f'os.execv({real!r}, [{real!r}, *sys.argv[1:]])\n'


def fake_executable(directory: Path, rules: object = None) -> Path:
    """A script that prints the rules for `--json rules` and fails otherwise."""
    path = directory / 'topology-checker'
    reply = json.dumps(json.dumps(RULES if rules is None else rules))
    return write_tool(
        path,
        '#!/usr/bin/env python3\n'
        'import sys\n'
        "if sys.argv[1:] == ['--json', 'rules']:\n"
        f'    print({reply})\n'
        '    sys.exit(0)\n'
        'print(\'{"error": "unsupported"}\', file=sys.stderr)\n'
        'sys.exit(2)\n',
    )


def script_executable(
    directory: Path,
    body: str,
    name: str = 'topology-checker',
    rules: object = None,
) -> Path:
    """A script for the system Python that answers `--json rules` and
    otherwise runs the body."""
    path = directory / name
    reply = json.dumps(json.dumps(RULES if rules is None else rules))
    return write_tool(
        path,
        '#!/usr/bin/env python3\n'
        'import sys\n'
        "if sys.argv[1:] == ['--json', 'rules']:\n"
        f'    print({reply})\n'
        '    sys.exit(0)\n' + body,
    )


# The algorithms of the provider when the tool lists the two RULES
FAKE_ALGORITHMS = {
    'line_must_not_have_dangles',
    'point_must_be_covered_by_lines',
    'merge_lines',
    'explode_lines',
    'check_rules',
    'check_definition',
}


def refresh_to_fake_tool(provider: t.Any) -> None:
    """Refreshes the algorithms, as the Options dialog does on OK, and waits
    until the provider has those of the tool that lists the two RULES: a tool
    that has not been asked is asked in a task."""
    provider.refreshAlgorithms()
    wait_for(
        lambda: {a.name() for a in provider.algorithms()} == FAKE_ALGORITHMS
    )


def probe_executable(
    directory: Path,
    name: str = 'topology-checker',
    rules: object = None,
    gate: Path | None = None,
    delay: float = 0.0,
    broken: bool = False,
) -> Path:
    """A script that lists its rules after a delay and once the gate exists,
    or fails to when it is broken. It runs nothing else."""
    path = directory / name
    reply = json.dumps(json.dumps(RULES if rules is None else rules))
    wait = (
        ''
        if gate is None
        else (
            f'    while not os.path.exists({str(gate)!r}):\n'
            '        time.sleep(0.02)\n'
        )
    )
    answer = (
        f'    print("{name} is broken", file=sys.stderr)\n    sys.exit(2)\n'
        if broken
        else f'    print({reply})\n    sys.exit(0)\n'
    )
    return write_tool(
        path,
        '#!/usr/bin/env python3\n'
        'import os, sys, time\n'
        "if sys.argv[1:] == ['--json', 'rules']:\n"
        f'    time.sleep({delay!r})\n' + wait + answer + 'sys.exit(2)\n',
    )


def counting_executable(
    directory: Path, log: Path, name: str = 'topology-checker'
) -> Path:
    """The real tool, behind a script that writes the arguments of each call
    to the log file, a line for each."""
    real = real_executable()
    assert real is not None, 'set TOPOLOGY_CHECKER'
    path = directory / name
    return write_tool(
        path,
        '#!/usr/bin/env python3\n'
        'import os, sys\n'
        f'with open({str(log)!r}, "a") as log:\n'
        '    log.write(" ".join(sys.argv[1:]) + "\\n")\n' + hand_over(real),
    )


def measuring_executable(
    directory: Path, log: Path, name: str = 'topology-checker'
) -> Path:
    """The real tool, behind a script that writes to the log file how many
    bytes of output each call gave, a line for each call."""
    real = real_executable()
    assert real is not None, 'set TOPOLOGY_CHECKER'
    path = directory / name
    return write_tool(
        path,
        '#!/usr/bin/env python3\n'
        'import subprocess, sys\n'
        f'command = [{real!r}, *sys.argv[1:]]\n'
        'done = subprocess.run(command, capture_output=True)\n'
        f'with open({str(log)!r}, "a") as log:\n'
        '    words = " ".join(sys.argv[1:])\n'
        '    log.write(words + " " + str(len(done.stdout)) + "\\n")\n'
        'sys.stdout.buffer.write(done.stdout)\n'
        'sys.stderr.buffer.write(done.stderr)\n'
        'sys.exit(done.returncode)\n',
    )


def output_sizes(log: Path, command: str) -> list[int]:
    """The bytes of output of each call of a measuring tool with the
    command."""
    if not log.exists():
        return []
    return [
        int(line.rsplit(' ', 1)[1])
        for line in log.read_text().splitlines()
        if command in line.split()
    ]


def calls(log: Path, command: str) -> list[str]:
    """The calls of a counting tool with the command: 'rules', 'validate'."""
    if not log.exists():
        return []
    return [
        line for line in log.read_text().splitlines() if command in line.split()
    ]


def process_alive(number: int) -> bool:
    """Whether a process of the system is there."""
    if sys.platform == 'win32':
        done = subprocess.run(
            ['tasklist', '/FI', f'PID eq {number}', '/NH'],
            capture_output=True,
            text=True,
            check=False,
        )
        return str(number) in done.stdout
    try:
        os.kill(number, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def gated_executable(
    directory: Path, gate: Path, started: Path, name: str = 'topology-checker'
) -> Path:
    """A tool that lists its rules at once. To check, it says that it began
    by creating the started file, with its process number in it, waits for the
    gate file to exist, and then is the real tool."""
    real = real_executable()
    assert real is not None, 'set TOPOLOGY_CHECKER'
    # Only a check waits: the other questions that the plugin asks the tool,
    # such as which formats it reads, are answered by the real tool at once
    body = (
        'import os, time\n'
        "if 'validate' in sys.argv:\n"
        f'    open({str(started)!r}, "w").write(str(os.getpid()))\n'
        f'    while not os.path.exists({str(gate)!r}):\n'
        '        time.sleep(0.02)\n' + hand_over(real)
    )
    return script_executable(directory, body, name)


# As a path from here, as some tests run the tool from another folder
NAMED = os.environ.get('TOPOLOGY_CHECKER')
NAMED_PATH = str(Path(NAMED).absolute()) if NAMED else None


def real_executable() -> str | None:
    """The tool named by TOPOLOGY_CHECKER, or the one on the PATH."""
    return NAMED_PATH or shutil.which('topology-checker')


def choose_engine(chosen: Engine) -> None:
    """Has the engine run the checks, as the user chooses it."""
    engine.choose(chosen)
    # A task reads the setting on a thread of its own, which sees only what
    # is in the file
    QgsSettings().sync()


# Runs a test, or each test of a module, once with the executable and once
# with the Python package
both_engines = pytest.mark.usefixtures('run_engine')
# Runs a test of such a module with the executable alone, as it is about the
# executable
executable_only = pytest.mark.parametrize(
    'run_engine', [Engine.EXECUTABLE], ids=['executable'], indirect=True
)


# Layers


def line_layer(*lines: str, name: str = 'lines') -> QgsVectorLayer:
    """A memory layer of WKT lines in a UTM CRS."""
    layer = QgsVectorLayer('LineString?crs=EPSG:32633', name, 'memory')
    add_features(layer, lines)
    return layer


def polygon_layer(*polygons: str, name: str = 'polygons') -> QgsVectorLayer:
    layer = QgsVectorLayer('Polygon?crs=EPSG:32633', name, 'memory')
    add_features(layer, polygons)
    return layer


def point_layer(*points: str, name: str = 'points') -> QgsVectorLayer:
    layer = QgsVectorLayer('Point?crs=EPSG:32633', name, 'memory')
    add_features(layer, points)
    return layer


def add_features(layer: QgsVectorLayer, wkts: t.Iterable[str]) -> None:
    features: list[QgsFeature] = []
    for wkt in wkts:
        feature = QgsFeature()
        feature.setGeometry(QgsGeometry.fromWkt(wkt))
        features.append(feature)
    layer.dataProvider().addFeatures(features)
    layer.updateExtents()


def save(layer: QgsVectorLayer, path: Path, driver: str, name: str) -> None:
    """Writes the layer to a file, as a layer of the given name."""
    options = QgsVectorFileWriter.SaveVectorOptions()
    options.driverName = driver
    options.layerName = name
    result = QgsVectorFileWriter.writeAsVectorFormatV3(
        layer, str(path), QgsProject.instance().transformContext(), options
    )
    assert result[0] == QgsVectorFileWriter.WriterError.NoError


# A layer keeps its file open as long as it is loaded, and Windows does not
# let a program delete a file that is open. So a test of a file that vanishes
# under a layer that is loaded cannot be made there
files_stay_open = pytest.mark.skipif(
    sys.platform == 'win32',
    reason='a file that a loaded layer has open cannot be deleted on Windows',
)


def remove_files(directory: Path, stem: str) -> None:
    """Deletes the files of a layer: the file and what goes with it."""
    for part in directory.glob(f'{stem}.*'):
        part.unlink()


def saved_layer(
    directory: Path,
    wkts: t.Iterable[str],
    name: str = 'lines',
    driver: str = 'GPKG',
) -> QgsVectorLayer:
    """A layer read from a file of its own, the usual layer of a project."""
    suffix = {'GPKG': '.gpkg', 'ESRI Shapefile': '.shp', 'GeoJSON': '.geojson'}
    path = directory / f'{name}{suffix[driver]}'
    save(line_layer(*wkts, name=name), path, driver, name)
    uri = f'{path}|layername={name}' if driver == 'GPKG' else str(path)
    layer = QgsVectorLayer(uri, name, 'ogr')
    assert layer.isValid()
    return layer


def saved_points(
    directory: Path, wkts: t.Iterable[str], name: str = 'stops'
) -> QgsVectorLayer:
    """A point layer read from a GeoPackage of its own."""
    path = directory / f'{name}.gpkg'
    save(point_layer(*wkts, name=name), path, 'GPKG', name)
    layer = QgsVectorLayer(f'{path}|layername={name}', name, 'ogr')
    assert layer.isValid()
    return layer


# Running algorithms and the tool


@dataclasses.dataclass
class Ran:
    results: dict[str, t.Any]
    ok: bool
    context: QgsProcessingContext
    log: str


@dataclasses.dataclass
class Started:
    """A threaded run that has begun; `ran()` is its result once it is over."""

    outcome: list[tuple[bool, dict[str, t.Any]]]
    context: QgsProcessingContext
    feedback: QgsProcessingFeedback
    # Keeps the task alive in Python while it runs
    task: QgsProcessingAlgRunnerTask

    @property
    def done(self) -> bool:
        return bool(self.outcome)

    def ran(self) -> Ran:
        wait_for(lambda: self.done)
        ok, results = self.outcome[0]
        return Ran(results, ok, self.context, self.feedback.textLog())


def start_algorithm(
    name: str, parameters: dict[str, t.Any], invalid_check: t.Any = None
) -> Started:
    """Starts a provider algorithm in a task thread, as the dock and the
    Processing dialog do, and returns at once."""
    algorithm = QgsApplication.processingRegistry().createAlgorithmById(
        f'{PROVIDER_ID}:{name}'
    )
    assert algorithm is not None
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    if invalid_check is not None:
        context.setInvalidGeometryCheck(invalid_check)
    feedback = QgsProcessingFeedback()
    task = QgsProcessingAlgRunnerTask(algorithm, parameters, context, feedback)
    outcome: list[tuple[bool, dict[str, t.Any]]] = []
    task.executed.connect(lambda ok, results: outcome.append((ok, results)))
    QgsApplication.taskManager().addTask(task)
    return Started(outcome, context, feedback, task)


def run_algorithm(
    name: str,
    parameters: dict[str, t.Any],
    threaded: bool = True,
    invalid_check: t.Any = None,
) -> Ran:
    """Runs a provider algorithm in a task thread, unless it is not
    threaded, and waits for the end."""
    if threaded:
        return start_algorithm(name, parameters, invalid_check).ran()
    algorithm = QgsApplication.processingRegistry().createAlgorithmById(
        f'{PROVIDER_ID}:{name}'
    )
    assert algorithm is not None
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    if invalid_check is not None:
        context.setInvalidGeometryCheck(invalid_check)
    feedback = QgsProcessingFeedback()
    results, ok = algorithm.run(parameters, context, feedback)
    return Ran(results, ok, context, feedback.textLog())


def error_layers() -> list[QgsVectorLayer]:
    """The layers of errors that a run put in the project."""
    return [
        layer
        for layer in QgsProject.instance().mapLayers().values()
        if isinstance(layer, QgsVectorLayer)
        and layer.fields().indexOf('violation_id') >= 0
    ]


def errors_layer(ran: Ran, name: str) -> QgsVectorLayer:
    """The layer of an output of a run: a layer of its context or a path."""
    value = str(ran.results[name])
    layer = ran.context.getMapLayer(value) or QgsVectorLayer(value, name, 'ogr')
    assert layer.isValid(), f'{name} is not a layer: {value}'
    return t.cast(QgsVectorLayer, layer)


def wait_for(condition: t.Callable[[], bool], timeout: float = 60.0) -> None:
    """Runs the event loop until the condition holds, as QGIS does.

    When it never holds, the stacks of all the threads say where a task is.
    """
    deadline = time.monotonic() + timeout
    while not condition():
        if time.monotonic() >= deadline:
            faulthandler.dump_traceback(file=sys.stderr, all_threads=True)
            raise AssertionError('timed out')
        QgsApplication.processEvents()
        time.sleep(0.01)


def write_toml(
    path: Path,
    layers: dict[str, tuple[str, str | None]],
    rules: list[tuple[str, str, str | None]],
    tolerance: float | None = None,
) -> Path:
    """A topology.toml: aliases of (path, layer name), and rules as (rule,
    layer, other)."""
    lines = [] if tolerance is None else [f'tolerance = {tolerance!r}', '']
    lines.append('[layers]')
    for alias, (source, name) in layers.items():
        entry = f'path = {json.dumps(source)}'
        if name is not None:
            entry += f', layer = {json.dumps(name)}'
        lines.append(f'{json.dumps(alias)} = {{ {entry} }}')
    for rule, layer, other in rules:
        lines += ['', '[[rules]]', f'rule = {json.dumps(rule)}']
        lines.append(f'layer = {json.dumps(layer)}')
        if other is not None:
            lines.append(f'other = {json.dumps(other)}')
    path.write_text('\n'.join(lines) + '\n', encoding='utf-8')
    return path


def cli_counts(
    executable: str, definition: Path, tolerance: float | None = None
) -> list[int]:
    """The errors of each rule of a definition, from the tool itself."""
    command = [executable, '--json', 'validate', str(definition)]
    if tolerance is not None:
        command += ['--tolerance', str(tolerance)]
    # From another folder, so that relative paths are the file's
    done = subprocess.run(
        command, capture_output=True, text=True, cwd='/', check=False
    )
    assert done.returncode in (0, 1), done.stderr
    report = json.loads(done.stdout)
    return [sum(rule['counts'].values()) for rule in report['rules']]


# The form of the dock


def listed_rules(dock: Dock) -> list[str]:
    """The names of the rules in the rule box, such as
    'line/must-not-have-dangles'."""
    return [
        dock.rule_box.itemData(index).name
        for index in range(dock.rule_box.count())
    ]


def choose_rule(dock: Dock, name: str, seconds: float = 10.0) -> None:
    """Picks the rule of the box. The tool may still be asked for its rules,
    so the rule is waited for."""
    deadline = time.monotonic() + seconds
    while name not in listed_rules(dock):
        assert time.monotonic() < deadline, (
            f'{name} is not in the rule box: {listed_rules(dock)}'
        )
        QApplication.processEvents()
        time.sleep(0.01)
    dock.rule_box.setCurrentIndex(listed_rules(dock).index(name))


def add(
    dock: Dock,
    name: str,
    layer: QgsVectorLayer,
    other: QgsVectorLayer | None = None,
) -> None:
    """Fills the form as a user does, the layer first, and clicks Add."""
    dock.input_box.setLayer(layer)
    choose_rule(dock, name)
    if other is not None:
        dock.other_box.setLayer(other)
    dock.add_button.click()


def click_run(dock: Dock) -> None:
    """Clicks Run and waits for the end of the task."""
    dock.run_button.click()
    wait_for(lambda: dock.task is None)


# The list of rules: layers that hold their rules


def layer_nodes(dock: Dock) -> list[QTreeWidgetItem]:
    tree = dock.rules_tree
    return [
        tree.topLevelItem(index) for index in range(tree.topLevelItemCount())
    ]


def rule_items(dock: Dock) -> list[QTreeWidgetItem]:
    """The rules of the list, in the order of the tree."""
    return [
        node.child(row)
        for node in layer_nodes(dock)
        for row in range(node.childCount())
    ]


def select_rules(dock: Dock, *indexes: int) -> None:
    """Selects rules of the list, by their place in it."""
    dock.rules_tree.clearSelection()
    items = rule_items(dock)
    for index in indexes:
        items[index].setSelected(True)
    # Without a command the call would drop the other selected items
    dock.rules_tree.setCurrentItem(
        items[indexes[-1]], 0, QItemSelectionModel.SelectionFlag.NoUpdate
    )


def checked(dock: Dock) -> list[bool]:
    """The checkbox of each rule of the list."""
    return [
        item.checkState(0) == Qt.CheckState.Checked for item in rule_items(dock)
    ]


def set_checked(dock: Dock, index: int, value: bool) -> None:
    """Ticks or clears the checkbox of a rule, as a click does."""
    state = Qt.CheckState.Checked if value else Qt.CheckState.Unchecked
    rule_items(dock)[index].setCheckState(0, state)


def queue_column(dock: Dock, index: int) -> list[str]:
    """What the list says of each rule: its name (0), its layer (1), its
    other layer (2) and its number of errors (3)."""
    texts: list[str] = []
    for item in rule_items(dock):
        name, _, other = item.text(0).partition(' with ')
        texts.append([name, item.parent().text(0), other, item.text(1)][index])
    return texts


# The results: rules, types of error, errors


def _walk(
    model: t.Any, parent: QModelIndex | None = None, depth: int = 0
) -> t.Iterator[tuple[int, QModelIndex]]:
    parent = parent if parent is not None else QModelIndex()
    for row in range(model.rowCount(parent)):
        index = model.index(row, 0, parent)
        yield depth, index
        yield from _walk(model, index, depth + 1)


def result_nodes(dock: Dock, depth: int) -> list[QModelIndex]:
    """The nodes of a level of the results: 0 rules, 1 types, 2 errors."""
    model = dock.results_view.model()
    return [index for level, index in _walk(model) if level == depth]


def node_text(index: QModelIndex) -> str:
    return str(index.data() or '')


def without_count(text: str) -> str:
    """'countries: Must not overlap (2)' as 'countries: Must not overlap'."""
    return re.sub(r' \(\d+\)$', '', text)


def without_layer(text: str) -> str:
    """'countries: Must not overlap' as 'Must not overlap'."""
    return text.partition(': ')[2] or text


def result_rows(dock: Dock) -> list[list[str]]:
    """The errors, as Rule, Error, Layer, Feature, Other layer, Other feature.

    The top node says the layer and the rule ('lakes: Must not overlap with
    countries (29)'), the next one the type of error ('Overlaps (29)'), and
    the leaf the features: 'rivers 12' or 'lakes 6 · countries 168'. The rule
    is given without the layer and the count, the type without its count.
    """
    rows: list[list[str]] = []
    for error in result_nodes(dock, 2):
        kind = error.parent()
        rule = kind.parent()
        first, _, second = node_text(error).partition(' · ')
        layer, _, feature = first.rpartition(' ')
        other, _, other_feature = second.rpartition(' ')
        rows.append(
            [
                without_layer(without_count(node_text(rule))),
                without_count(node_text(kind)),
                layer,
                feature,
                other,
                other_feature,
            ]
        )
    return rows


def shown(dock: Dock) -> int:
    """The number of errors in the results of the dock."""
    return len(result_nodes(dock, 2))


def cell(dock: Dock, row: int, column: int) -> str:
    """The text of a cell of the results, as result_rows gives it."""
    return result_rows(dock)[row][column]


def select_error(dock: Dock, row: int) -> None:
    """Selects an error of the results, as a click on it does."""
    index = result_nodes(dock, 2)[row]
    dock.results_view.setCurrentIndex(index)


def select_result(dock: Dock, text: str) -> QModelIndex:
    """Selects the rule or type of error whose text has the text in it."""
    for depth in (0, 1):
        for index in result_nodes(dock, depth):
            if text in node_text(index):
                dock.results_view.setCurrentIndex(index)
                return index
    raise AssertionError(f'no node {text!r} in the results')


# The message bar


def message_text(bar: t.Any) -> str:
    """Everything a user can read in the message bar."""
    texts = [
        text
        for level in (
            Qgis.MessageLevel.Critical,
            Qgis.MessageLevel.Warning,
            Qgis.MessageLevel.Info,
            Qgis.MessageLevel.Success,
        )
        for text in bar.get_messages(level)
    ]
    texts += [label.text() for label in bar.findChildren(QLabel)]
    return '\n'.join(texts)


def duration_of(call: tuple[tuple[t.Any, ...], dict[str, t.Any]]) -> int:
    """The seconds a message stays, from the arguments of `pushMessage`
    (title, text, level, duration); 0 or less is until the user closes it."""
    args, kwargs = call
    if len(args) > 3:
        return int(args[3])
    return int(kwargs.get('duration', -1))
