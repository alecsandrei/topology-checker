"""The list of rules to run: adding, removing, running them together."""

from __future__ import annotations

import collections.abc as c
from pathlib import Path

from qgis.core import QgsProject, QgsVectorLayer

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    both_engines,
    cell,
    choose_rule,
    click_run,
    executable_only,
    layer_nodes,
    point_layer,
    queue_column,
    saved_layer,
    script_executable,
    select_rules,
    shown,
    wait_for,
)

pytestmark = both_engines

DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)


def lines(tmp_path: Path, wkts: tuple[str, ...], name: str) -> QgsVectorLayer:
    layer = saved_layer(tmp_path, wkts, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def points(*wkts: str, name: str = 'points') -> QgsVectorLayer:
    layer = point_layer(*wkts, name=name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def groups() -> list[str]:
    root = QgsProject.instance().layerTreeRoot()
    return [
        g.name() for g in root.findGroups() if g.name().startswith('Topology')
    ]


def test_adding_a_rule_adds_it_under_its_layer(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    assert len(dock.entries) == 1
    assert queue_column(dock, 0) == ['Must not have dangles']
    assert queue_column(dock, 1) == ['roads']
    assert dock.run_button.text() == 'Run 1 rule'


def test_the_same_rule_on_the_same_layer_is_not_added_twice(
    dock: Dock, tmp_path: Path
):
    layer = lines(tmp_path, CHAIN, 'roads')
    add(dock, DANGLES, layer)
    add(dock, DANGLES, layer)
    assert len(dock.entries) == 1


def test_the_same_rule_on_two_layers(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    add(dock, DANGLES, lines(tmp_path, RING, 'rails'))
    assert len(dock.entries) == 2
    assert len(layer_nodes(dock)) == 2
    assert dock.run_button.text() == 'Run 2 rules'


def test_adding_a_two_layer_rule_without_the_second_layer(
    dock: Dock, tmp_path: Path
):
    stops = points('POINT(5 0)')
    dock.input_box.setLayer(stops)
    choose_rule(dock, COVERED)
    dock.other_box.setCurrentIndex(-1)
    dock.add_button.click()
    assert dock.entries == []


def test_an_empty_list_cannot_be_run(dock: Dock):
    assert not dock.run_button.isEnabled()
    assert dock.run_button.text() == 'Run'
    dock.run_button.click()
    assert dock.task is None


def test_removing_the_selected_rule(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    add(dock, DANGLES, lines(tmp_path, RING, 'rails'))
    select_rules(dock, 0)
    assert dock.remove_button.isEnabled()
    dock.remove_button.click()
    assert queue_column(dock, 1) == ['rails']


def test_nothing_selected_removes_nothing(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    assert not dock.remove_button.isEnabled()
    dock.remove_rules()
    assert len(dock.entries) == 1


def test_clearing_the_list(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    dock.clear_button.click()
    assert dock.entries == []
    assert dock.rules_tree.topLevelItemCount() == 0
    assert not dock.run_button.isEnabled()
    assert not dock.clear_button.isEnabled()


def test_a_layer_removed_from_the_project_blocks_the_run(
    dock: Dock, tmp_path: Path
):
    roads = lines(tmp_path, CHAIN, 'roads')
    add(dock, DANGLES, roads)
    QgsProject.instance().removeMapLayer(roads.id())
    dock.run_button.click()
    assert dock.task is None
    assert shown(dock) == 0


def test_two_rules_on_two_layers_run_as_one_task(dock: Dock, tmp_path: Path):
    roads = lines(tmp_path, CHAIN, 'roads')
    rails = lines(tmp_path, RING, 'rails')
    add(dock, DANGLES, roads)
    add(dock, DANGLES, rails)
    dock.run_button.click()
    assert dock.task is not None
    wait_for(lambda: dock.task is None)
    assert queue_column(dock, 3) == ['2', '0']
    assert shown(dock) == 2
    names = {cell(dock, row, 0) for row in range(2)}
    assert names == {'Must not have dangles'}


def test_two_different_rules_name_themselves_in_the_results(
    dock: Dock, tmp_path: Path
):
    roads = lines(tmp_path, CHAIN, 'roads')
    stops = points('POINT(5 0)', 'POINT(5 5)')
    add(dock, DANGLES, roads)
    add(dock, COVERED, stops, roads)
    click_run(dock)
    assert sorted(queue_column(dock, 3)) == ['1', '2']
    names = {cell(dock, row, 0) for row in range(3)}
    assert names == {
        'Must not have dangles',
        'Must be covered by lines with roads',
    }
    # The layer of each error is the layer that was checked
    layers = {cell(dock, row, 2) for row in range(3)}
    assert layers == {'roads', 'points'}


def test_a_rerun_leaves_one_group(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    click_run(dock)
    click_run(dock)
    assert groups() == ['Topology errors']
    assert shown(dock) == 2


def test_a_rerun_without_errors_removes_the_group(dock: Dock, tmp_path: Path):
    roads = lines(tmp_path, CHAIN, 'roads')
    add(dock, DANGLES, roads)
    click_run(dock)
    assert groups() == ['Topology errors']
    dock.clear_rules()
    add(dock, DANGLES, lines(tmp_path, RING, 'rails'))
    click_run(dock)
    assert groups() == []
    assert shown(dock) == 0


def test_the_list_survives_a_run(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    click_run(dock)
    assert len(dock.entries) == 1
    assert queue_column(dock, 0) == ['Must not have dangles']


@executable_only
def test_the_list_cannot_change_during_a_run(
    dock: Dock,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
):
    configure(script_executable(tmp_path, 'import time\ntime.sleep(60)\n'))
    dock.refresh()
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    dock.run_button.click()
    assert dock.task is not None
    assert not dock.add_button.isEnabled()
    assert not dock.remove_button.isEnabled()
    assert not dock.clear_button.isEnabled()
    dock.clear_rules()
    dock.remove_rules()
    assert len(dock.entries) == 1
    dock.run_button.click()
    wait_for(lambda: dock.task is None, timeout=20)
    assert queue_column(dock, 3) == ['']
    assert dock.add_button.isEnabled()


def test_the_errors_column_is_emptied_by_the_next_run(
    dock: Dock, tmp_path: Path
):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    click_run(dock)
    assert queue_column(dock, 3) == ['2']
    dock.run_button.click()
    assert queue_column(dock, 3) == ['']
    wait_for(lambda: dock.task is None)
    assert queue_column(dock, 3) == ['2']


def test_a_removed_layer_does_not_break_the_next_add(
    dock: Dock, tmp_path: Path
):
    roads = lines(tmp_path, CHAIN, 'roads')
    add(dock, DANGLES, roads)
    QgsProject.instance().removeMapLayer(roads.id())
    add(dock, DANGLES, lines(tmp_path, RING, 'rails'))
    assert len(dock.entries) == 2


def test_a_cleared_project_has_nothing_to_run(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    QgsProject.instance().clear()
    assert dock.entries == []
    assert not dock.run_button.isEnabled()
    dock.run_button.click()
    assert dock.task is None


def test_hiding_and_showing_the_dock_keeps_the_list(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    dock.show()
    dock.hide()
    dock.show()
    assert len(dock.entries) == 1
    assert queue_column(dock, 1) == ['roads']
    assert dock.run_button.isEnabled()


def test_showing_the_dock_keeps_the_results(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN, 'roads'))
    click_run(dock)
    dock.hide()
    dock.show()
    assert shown(dock) == 2
    assert queue_column(dock, 3) == ['2']
