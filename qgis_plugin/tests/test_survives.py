"""What the user must not lose without meaning to: the list of rules, their
checkboxes, the tolerance and the results of the last run.

The same state is read before and after each thing that can happen to the
dock: the tabs, floating it, the toolbar button, the Processing provider
that loads its algorithms again, the project that is saved, a layer that is
renamed or gets another CRS.

Two choices are made here. A renamed layer is renamed in the list at once,
and the results keep the names of the run that made them. A layer that gets
another CRS leaves the list and the results as they are, because they are
what the last run found; the next run uses the new CRS, and the unit of the
tolerance follows the first layer of the list."""

from __future__ import annotations

import collections.abc as c
import dataclasses
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsCoordinateReferenceSystem, QgsProject, QgsVectorLayer
from qgis.gui import QgisInterface
from qgis.PyQt.QtWidgets import QApplication, QMainWindow

from topology_rules.panel.dock import Dock
from topology_rules.plugin import TopologyRules

from .helpers import (
    add,
    checked,
    click_run,
    node_text,
    point_layer,
    queue_column,
    result_nodes,
    saved_layer,
    script_executable,
    set_checked,
    shown,
    wait_for,
)

DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


@dataclasses.dataclass
class Layers:
    roads: QgsVectorLayer
    stops: QgsVectorLayer


@pytest.fixture
def plugin(
    qgis_iface: QgisInterface,
    window: QMainWindow,
    executable: str,
    qgis_processing: None,
) -> c.Iterator[TopologyRules]:
    """The plugin in a window, wired as QGIS does it."""
    # A dock tells that it is shown only when its window is shown
    window.resize(1000, 900)
    window.show()
    plugin = TopologyRules(qgis_iface)
    plugin.initGui()
    plugin.dock.resize(300, 900)
    plugin.dock.show()
    QApplication.processEvents()
    yield plugin
    plugin.unload()


@pytest.fixture
def dock(plugin: TopologyRules) -> Dock:
    return plugin.dock


@pytest.fixture
def layers(tmp_path: Path) -> Layers:
    roads = saved_layer(tmp_path, CHAIN, 'roads')
    stops = point_layer('POINT(5 0)', 'POINT(5 5)', name='stops')
    for layer in (roads, stops):
        QgsProject.instance().addMapLayer(layer)
    return Layers(roads, stops)


@pytest.fixture
def worked(dock: Dock, layers: Layers) -> Dock:
    """Two rules that were run, with a tolerance: the state that must be
    kept."""
    add(dock, DANGLES, layers.roads)
    add(dock, COVERED, layers.stops, layers.roads)
    dock.tolerance.setValue(0.25)
    click_run(dock)
    return dock


def state(dock: Dock) -> dict[str, t.Any]:
    """Everything that the user would notice if it were gone."""
    return {
        'rules': queue_column(dock, 0),
        'layers': queue_column(dock, 1),
        'others': queue_column(dock, 2),
        'counts': queue_column(dock, 3),
        'checked': checked(dock),
        'errors': shown(dock),
        'nodes': [node_text(node) for node in result_nodes(dock, 0)],
        'tolerance': dock.tolerance.value(),
        'run': dock.run_button.text(),
    }


def test_the_state_to_keep_is_not_empty(worked: Dock):
    now = state(worked)
    assert now['rules'] == ['Must not have dangles', 'Must be covered by lines']
    assert now['errors'] == 3
    assert now['counts'] == ['2', '1']
    assert now['checked'] == [True, True]
    assert now['tolerance'] == 0.25
    assert now['run'] == 'Run 2 rules'


# A checkbox


def test_unchecking_a_rule_keeps_the_results(worked: Dock):
    before = shown(worked)
    set_checked(worked, 1, False)
    QApplication.processEvents()
    assert shown(worked) == before == 3
    assert queue_column(worked, 3) == ['2', '1']


def test_a_checkbox_that_was_cleared_survives_what_follows(
    plugin: TopologyRules, worked: Dock
):
    set_checked(worked, 1, False)
    before = state(worked)
    assert before['checked'] == [True, False]
    worked.tabs.setCurrentIndex(0)
    worked.setFloating(True)
    worked.setFloating(False)
    plugin.action.setChecked(False)
    plugin.action.setChecked(True)
    plugin.provider.refreshAlgorithms()
    QApplication.processEvents()
    assert state(worked) == before


# The tabs


def test_switching_the_tabs_keeps_the_state(worked: Dock):
    before = state(worked)
    for index in (0, 1, 0, 1, 1, 0):
        worked.tabs.setCurrentIndex(index)
        QApplication.processEvents()
    assert state(worked) == before


def test_switching_the_tabs_during_a_run_keeps_the_list(
    plugin: TopologyRules,
    layers: Layers,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
):
    dock = plugin.dock
    configure(script_executable(tmp_path, 'import time\ntime.sleep(60)\n'))
    plugin.provider.refreshAlgorithms()
    add(dock, DANGLES, layers.roads)
    before = queue_column(dock, 0), queue_column(dock, 1)
    dock.run_button.click()
    for index in (1, 0, 1, 0):
        dock.tabs.setCurrentIndex(index)
        QApplication.processEvents()
    assert dock.task is not None
    dock.run_button.click()
    wait_for(lambda: dock.task is None, timeout=20)
    assert (queue_column(dock, 0), queue_column(dock, 1)) == before


def test_the_run_goes_on_when_the_tabs_are_switched(dock: Dock, layers: Layers):
    add(dock, DANGLES, layers.roads)
    dock.run_button.click()
    dock.tabs.setCurrentIndex(0)
    dock.tabs.setCurrentIndex(1)
    wait_for(lambda: dock.task is None)
    assert shown(dock) == 2
    assert queue_column(dock, 3) == ['2']


# Floating and docking


def test_floating_and_docking_again_keeps_the_state(worked: Dock):
    before = state(worked)
    worked.setFloating(True)
    QApplication.processEvents()
    assert state(worked) == before
    worked.setFloating(False)
    QApplication.processEvents()
    assert state(worked) == before


def test_the_dock_can_run_again_after_floating(worked: Dock):
    worked.setFloating(True)
    QApplication.processEvents()
    worked.setFloating(False)
    click_run(worked)
    assert shown(worked) == 3


# The toolbar button


def test_toggling_the_toolbar_button_keeps_the_state(
    plugin: TopologyRules, worked: Dock
):
    before = state(worked)
    plugin.action.setChecked(False)
    QApplication.processEvents()
    assert worked.isHidden()
    plugin.action.setChecked(True)
    QApplication.processEvents()
    assert not worked.isHidden()
    assert state(worked) == before


def test_closing_the_dock_with_its_button_keeps_the_state(
    plugin: TopologyRules, worked: Dock
):
    before = state(worked)
    for _ in range(3):
        plugin.action.trigger()
        QApplication.processEvents()
    assert state(worked) == before


# The Processing provider


def test_the_provider_loading_its_algorithms_again_keeps_the_state(
    plugin: TopologyRules, worked: Dock
):
    before = state(worked)
    plugin.provider.refreshAlgorithms()
    QApplication.processEvents()
    assert state(worked) == before


def test_the_same_executable_set_again_keeps_the_state(
    plugin: TopologyRules,
    worked: Dock,
    configure: c.Callable[[str | Path], None],
    executable: str,
):
    before = state(worked)
    # What the options dialog does on OK with the path that was there
    configure(executable)
    plugin.provider.refreshAlgorithms()
    QApplication.processEvents()
    assert state(worked) == before


def test_the_form_still_lists_rules_after_the_provider_refreshed(
    plugin: TopologyRules, worked: Dock, layers: Layers
):
    plugin.provider.refreshAlgorithms()
    wait_for(lambda: worked.rule_box.count() > 0)
    worked.input_box.setLayer(layers.roads)
    assert worked.rule_box.count() > 0
    assert worked.add_button.isEnabled()


def test_a_run_after_the_provider_refreshed_works(
    plugin: TopologyRules, worked: Dock
):
    plugin.provider.refreshAlgorithms()
    click_run(worked)
    assert shown(worked) == 3


# The project


def test_saving_the_project_keeps_the_state(worked: Dock, tmp_path: Path):
    before = state(worked)
    assert QgsProject.instance().write(str(tmp_path / 'project.qgz'))
    QApplication.processEvents()
    assert state(worked) == before


# A layer that changes


def test_renaming_a_layer_renames_it_in_the_list_and_keeps_the_rest(
    worked: Dock, layers: Layers
):
    before = state(worked)
    layers.roads.setName('main roads')
    after = state(worked)
    assert after['layers'] == ['main roads', 'stops']
    for key in ('rules', 'counts', 'checked', 'errors', 'nodes', 'tolerance'):
        assert after[key] == before[key], key


def test_the_results_keep_the_names_of_their_run_after_a_rename(
    worked: Dock, layers: Layers
):
    nodes = [node_text(node) for node in result_nodes(worked, 0)]
    layers.roads.setName('main roads')
    assert [node_text(node) for node in result_nodes(worked, 0)] == nodes


def test_a_run_after_a_rename_uses_the_new_name(worked: Dock, layers: Layers):
    layers.roads.setName('main roads')
    click_run(worked)
    texts = [node_text(node) for node in result_nodes(worked, 0)]
    assert any(text.startswith('main roads: ') for text in texts)


def test_a_new_crs_keeps_the_list_and_the_results(worked: Dock, layers: Layers):
    before = state(worked)
    layers.stops.setCrs(QgsCoordinateReferenceSystem('EPSG:4326'))
    QApplication.processEvents()
    after = state(worked)
    for key in before:
        if key != 'tolerance':
            assert after[key] == before[key], key


def test_a_new_crs_on_the_first_layer_changes_the_unit_of_the_tolerance(
    dock: Dock, layers: Layers
):
    add(dock, DANGLES, layers.roads)
    assert dock.tolerance.suffix() == ' m'
    layers.roads.setCrs(QgsCoordinateReferenceSystem('EPSG:4326'))
    QApplication.processEvents()
    assert dock.tolerance.suffix() == ' °'


def test_a_run_after_a_new_crs_works(worked: Dock, layers: Layers):
    layers.stops.setCrs(QgsCoordinateReferenceSystem('EPSG:32633'))
    click_run(worked)
    assert queue_column(worked, 3)[0] == '2'
