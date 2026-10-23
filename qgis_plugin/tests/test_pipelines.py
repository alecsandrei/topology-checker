"""The dock end to end, the way users click: real task, real executable."""

from __future__ import annotations

from pathlib import Path

from qgis.core import QgsApplication, QgsProject, QgsVectorLayer

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    both_engines,
    click_run,
    saved_layer,
    select_error,
    shown,
    wait_for,
)

pytestmark = both_engines

DANGLES = 'line/must-not-have-dangles'
# Two lines meeting at a node: the free end of each is a dangle
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)


def project_layer(
    tmp_path: Path, wkts: tuple[str, ...], name: str = 'lines'
) -> QgsVectorLayer:
    layer = saved_layer(tmp_path, wkts, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def error_groups() -> list[str]:
    root = QgsProject.instance().layerTreeRoot()
    return [
        g.name() for g in root.findGroups() if g.name().startswith('Topology')
    ]


def test_run_lists_the_errors(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_layer(tmp_path, CHAIN))
    click_run(dock)
    assert shown(dock) == 2
    assert dock.run_button.isEnabled()


def test_run_on_clean_data_lists_nothing(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_layer(tmp_path, RING))
    click_run(dock)
    assert shown(dock) == 0
    assert dock.run_button.isEnabled()
    assert error_groups() == []


def test_every_row_zooms_to_its_error(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_layer(tmp_path, CHAIN))
    click_run(dock)
    for row in range(shown(dock)):
        select_error(dock, row)
    assert shown(dock) == 2


def remove_error_layers(keep: object) -> None:
    project = QgsProject.instance()
    for other in list(project.mapLayers().values()):
        if other.id() != keep:
            project.removeMapLayer(other.id())


def test_the_errors_go_with_their_layers(dock: Dock, tmp_path: Path):
    layer = project_layer(tmp_path, CHAIN)
    add(dock, DANGLES, layer)
    click_run(dock)
    remove_error_layers(layer.id())
    assert shown(dock) == 0


def test_a_selected_error_whose_layer_was_removed(dock: Dock, tmp_path: Path):
    layer = project_layer(tmp_path, CHAIN)
    add(dock, DANGLES, layer)
    click_run(dock)
    select_error(dock, 0)
    remove_error_layers(layer.id())
    for action in dock.results_view.actions():
        action.trigger()
    dock.select_button.click()
    assert shown(dock) == 0


def test_a_selected_error_after_a_new_project(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_layer(tmp_path, CHAIN))
    click_run(dock)
    select_error(dock, 0)
    QgsProject.instance().clear()
    for action in dock.results_view.actions():
        action.trigger()
    assert shown(dock) == 0


def test_run_while_running_is_ignored(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_layer(tmp_path, CHAIN))
    dock.run_button.click()
    dock.run_rules()
    wait_for(lambda: dock.task is None)
    assert shown(dock) == 2


def test_hiding_the_dock_during_a_run(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_layer(tmp_path, CHAIN))
    dock.run_button.click()
    dock.hide()
    dock.show()
    wait_for(lambda: dock.task is None)
    assert shown(dock) == 2


def test_removing_the_input_layer_during_a_run(dock: Dock, tmp_path: Path):
    layer = project_layer(tmp_path, CHAIN)
    add(dock, DANGLES, layer)
    dock.run_button.click()
    QgsProject.instance().removeMapLayer(layer.id())
    wait_for(lambda: dock.task is None)
    assert dock.run_button.isEnabled() or not dock.entries


def test_the_task_is_gone_from_the_manager(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_layer(tmp_path, CHAIN))
    click_run(dock)
    wait_for(lambda: not QgsApplication.taskManager().activeTasks())
