"""The rule of the threads: what a task needs from QGIS is taken on the main
thread when it is prepared, and the thread of the task only reads it."""

from __future__ import annotations

import collections.abc as c
import json
import typing as t
from pathlib import Path

import pytest
from qgis.core import (
    QgsApplication,
    QgsFeature,
    QgsGeometry,
    QgsProcessingAlgorithm,
    QgsProcessingUtils,
    QgsProject,
    QgsSettings,
    QgsVectorLayer,
)
from qgis.PyQt.QtCore import QThread
from qgis.PyQt.QtWidgets import QApplication

from topology_rules.core import binary
from topology_rules.enums import Setting

from .helpers import (
    line_layer,
    refresh_to_fake_tool,
    run_algorithm,
    saved_layer,
    saved_points,
    script_executable,
    start_algorithm,
    wait_for,
    write_toml,
)

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
Call = tuple[str, bool]


@pytest.fixture(autouse=True)
def ready(executable: str, provider: object) -> None:
    """The real executable and the provider, as in a QGIS session."""


@pytest.fixture
def calls(monkeypatch: pytest.MonkeyPatch) -> list[Call]:
    """Records, for each call that must stay on the main thread, whether it
    was made there. The calls still do what they did."""
    recorded: list[Call] = []

    def on_main() -> bool:
        app = QApplication.instance()
        return QThread.currentThread() == app.thread()

    def wrap(owner: t.Any, name: str, label: str, static: bool = False):
        original = getattr(owner, name)

        def spy(*args: t.Any, **kwargs: t.Any) -> t.Any:
            recorded.append((label, on_main()))
            return original(*args, **kwargs)

        monkeypatch.setattr(owner, name, staticmethod(spy) if static else spy)

    wrap(QgsProcessingAlgorithm, 'parameterAsSource', 'parameterAsSource')
    wrap(
        QgsProcessingUtils,
        'variantToSource',
        'variantToSource',
        static=True,
    )
    wrap(
        QgsProcessingUtils,
        'mapLayerFromString',
        'mapLayerFromString',
        static=True,
    )
    wrap(binary, 'find_executable', 'find_executable')
    wrap(binary, 'rules', 'rules')

    original_value = QgsSettings.value

    def value(self: QgsSettings, key: str, *args: t.Any, **kwargs: t.Any):
        if key == Setting.EXECUTABLE.value:
            recorded.append(('settings', on_main()))
        return original_value(self, key, *args, **kwargs)

    monkeypatch.setattr(QgsSettings, 'value', value)
    return recorded


def lines(tmp_path: Path, name: str = 'roads') -> QgsVectorLayer:
    layer = saved_layer(tmp_path, CHAIN, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def run_rule(tmp_path: Path):
    layer = lines(tmp_path)
    return run_algorithm('line_must_not_have_dangles', {'INPUT': layer.id()})


def run_check_rules(tmp_path: Path):
    layer = lines(tmp_path)
    rules = [{'rule': DANGLES, 'layer': layer.id(), 'other': None}]
    return run_algorithm(
        'check_rules',
        {'RULES': json.dumps(rules), 'OUTPUT': str(tmp_path / 'e.gpkg')},
    )


def run_check_definition(tmp_path: Path):
    data = tmp_path / 'data'
    data.mkdir()
    saved_layer(data, CHAIN, 'roads')
    saved_points(data, ['POINT(5 0)'])
    definition = write_toml(
        tmp_path / 'topology.toml',
        {'roads': ('data/roads.gpkg', 'roads')},
        [(DANGLES, 'roads', None)],
    )
    return run_algorithm('check_definition', {'DEFINITION': str(definition)})


def run_utility(tmp_path: Path):
    layer = lines(tmp_path)
    return run_algorithm(
        'merge_lines', {'INPUT': layer.id(), 'OUTPUT': 'memory:merged'}
    )


RUNS: dict[str, c.Callable[[Path], t.Any]] = {
    'rule': run_rule,
    'check_rules': run_check_rules,
    'check_definition': run_check_definition,
    'utility': run_utility,
}


@pytest.mark.parametrize('kind', RUNS)
def test_a_task_calls_qgis_only_on_the_main_thread(
    kind: str, tmp_path: Path, calls: list[Call]
):
    ran = RUNS[kind](tmp_path)
    assert ran.ok, ran.log
    off_thread = [label for label, main in calls if not main]
    assert off_thread == []


@pytest.mark.parametrize('kind', ['rule', 'check_rules', 'utility'])
def test_a_task_does_take_its_layers_while_it_is_prepared(
    kind: str, tmp_path: Path, calls: list[Call]
):
    # Without this the test above would pass for a task that took nothing
    RUNS[kind](tmp_path)
    assert calls
    assert all(main for _, main in calls)


# The executable setting is read when the run starts


def marker(directory: Path, name: str) -> Path:
    """A tool that lists the rules, and fails every check with its name."""
    reply = json.dumps(json.dumps({'error': name}))
    return script_executable(
        directory,
        f'print({reply}, file=sys.stderr)\nsys.exit(2)\n',
        name=name,
    )


@pytest.mark.parametrize('kind', ['rule', 'check_rules'])
def test_a_run_uses_the_executable_set_just_before_it_starts(
    kind: str,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    provider: t.Any,
):
    first = marker(tmp_path, 'tool-first')
    second = marker(tmp_path, 'tool-second')
    configure(first)
    refresh_to_fake_tool(provider)
    before = RUNS[kind](tmp_path)
    assert 'tool-first' in before.log
    configure(second)
    after = RUNS[kind](tmp_path)
    assert 'tool-second' in after.log
    assert 'tool-first' not in after.log


# Layers edited while the tasks run


def alternate(tmp_path: Path) -> tuple[QgsVectorLayer, QgsVectorLayer]:
    on_file = lines(tmp_path, 'on_file')
    in_memory = line_layer(*CHAIN, name='in_memory')
    QgsProject.instance().addMapLayer(in_memory)
    return on_file, in_memory


def edit(on_file: QgsVectorLayer, in_memory: QgsVectorLayer, step: int) -> None:
    """Changes both layers on the main thread: the data of the memory layer,
    and the edit buffer of the other."""
    feature = QgsFeature(in_memory.fields())
    feature.setGeometry(QgsGeometry.fromWkt(f'LINESTRING(0 {50 + step}, 1 0)'))
    provider = in_memory.dataProvider()
    assert provider is not None
    ok, added = provider.addFeatures([feature])
    assert ok
    provider.deleteFeatures([added[0].id()])
    if on_file.isEditable():
        on_file.rollBack()
    else:
        on_file.startEditing()
        added_feature = QgsFeature(on_file.fields())
        added_feature.setGeometry(QgsGeometry.fromWkt('LINESTRING(0 70, 1 70)'))
        on_file.addFeature(added_feature)


@pytest.mark.slow
def test_many_runs_while_the_layers_are_edited(tmp_path: Path):
    on_file, in_memory = alternate(tmp_path)
    for index in range(50):
        layer = on_file if index % 2 else in_memory
        started = start_algorithm(
            'line_must_not_have_dangles', {'INPUT': layer.id()}
        )
        step = 0
        while not started.done:
            edit(on_file, in_memory, step)
            step += 1
            QApplication.processEvents()
            QThread.msleep(2)
            assert step < 20_000, f'run {index} did not end'
        assert started.outcome[0][0], started.feedback.textLog()
    if on_file.isEditable():
        on_file.rollBack()
    wait_for(lambda: not QgsApplication.taskManager().activeTasks())
