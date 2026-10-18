"""The list of rules and the tolerance are saved with the project."""

from __future__ import annotations

from pathlib import Path

from qgis.core import QgsProject, QgsVectorLayer

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    checked,
    layer_nodes,
    queue_column,
    saved_layer,
    saved_points,
    set_checked,
    wait_for,
)

DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


def lines(tmp_path: Path, name: str) -> QgsVectorLayer:
    layer = saved_layer(tmp_path, CHAIN, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def filled(dock: Dock, tmp_path: Path) -> None:
    """Three rules, the second off, with a tolerance."""
    roads = lines(tmp_path, 'roads')
    rails = lines(tmp_path, 'rails')
    stops = saved_points(tmp_path, ['POINT(5 0)', 'POINT(5 5)'])
    QgsProject.instance().addMapLayer(stops)
    add(dock, DANGLES, roads)
    add(dock, DANGLES, rails)
    add(dock, COVERED, stops, roads)
    set_checked(dock, 1, False)
    dock.tolerance.setValue(0.5)


def save_and_reload(path: Path) -> None:
    project = QgsProject.instance()
    assert project.write(str(path))
    project.clear()
    assert project.read(str(path))


def test_the_list_is_restored_with_the_project(dock: Dock, tmp_path: Path):
    filled(dock, tmp_path)
    before = [queue_column(dock, index) for index in range(3)]
    save_and_reload(tmp_path / 'project.qgz')
    assert [queue_column(dock, index) for index in range(3)] == before


def test_the_order_of_the_rows_is_kept(dock: Dock, tmp_path: Path):
    filled(dock, tmp_path)
    save_and_reload(tmp_path / 'project.qgz')
    assert queue_column(dock, 0) == [
        'Must not have dangles',
        'Must not have dangles',
        'Must be covered by lines',
    ]
    assert queue_column(dock, 1) == ['roads', 'rails', 'stops']
    assert queue_column(dock, 2) == ['', '', 'roads']


def test_the_checkboxes_are_restored(dock: Dock, tmp_path: Path):
    filled(dock, tmp_path)
    save_and_reload(tmp_path / 'project.qgz')
    assert checked(dock) == [True, False, True]
    assert dock.run_button.text() == 'Run 2 rules'


def test_the_tolerance_is_restored(dock: Dock, tmp_path: Path):
    filled(dock, tmp_path)
    save_and_reload(tmp_path / 'project.qgz')
    assert dock.tolerance.value() == 0.5


def test_a_restored_list_can_be_run(dock: Dock, tmp_path: Path):
    filled(dock, tmp_path)
    save_and_reload(tmp_path / 'project.qgz')
    dock.run_button.click()
    wait_for(lambda: dock.task is None)
    assert queue_column(dock, 3) == ['2', '', '1']


def test_a_cleared_project_has_an_empty_list(dock: Dock, tmp_path: Path):
    filled(dock, tmp_path)
    QgsProject.instance().clear()
    assert dock.entries == []
    assert dock.rules_tree.topLevelItemCount() == 0


def test_a_cleared_project_has_the_default_tolerance(
    dock: Dock, tmp_path: Path
):
    filled(dock, tmp_path)
    QgsProject.instance().clear()
    assert dock.tolerance.value() == dock.tolerance.minimum()


def test_a_project_saved_without_a_list_restores_none(
    dock: Dock, tmp_path: Path
):
    path = tmp_path / 'empty.qgz'
    assert QgsProject.instance().write(str(path))
    filled(dock, tmp_path)
    assert QgsProject.instance().read(str(path))
    assert dock.entries == []
    assert dock.tolerance.value() == dock.tolerance.minimum()


def test_a_row_whose_layer_is_missing_after_reading_shows_the_warning(
    dock: Dock, tmp_path: Path
):
    roads = lines(tmp_path, 'roads')
    lines(tmp_path, 'rails')
    add(dock, DANGLES, roads)
    add(dock, DANGLES, QgsProject.instance().mapLayersByName('rails')[0])
    QgsProject.instance().removeMapLayer(roads.id())
    save_and_reload(tmp_path / 'project.qgz')
    first, second = layer_nodes(dock)
    assert not first.icon(0).isNull()
    assert first.toolTip(0)
    # The node of a layer that is there has the icon of the layer
    assert first.icon(0).cacheKey() != second.icon(0).cacheKey()
    assert first.toolTip(0) != second.toolTip(0)


def test_a_row_whose_layer_is_missing_blocks_the_run(
    dock: Dock, tmp_path: Path
):
    roads = lines(tmp_path, 'roads')
    add(dock, DANGLES, roads)
    QgsProject.instance().removeMapLayer(roads.id())
    save_and_reload(tmp_path / 'project.qgz')
    dock.run_button.click()
    assert dock.task is None


def test_reading_another_project_replaces_the_list(dock: Dock, tmp_path: Path):
    filled(dock, tmp_path)
    first = tmp_path / 'first.qgz'
    assert QgsProject.instance().write(str(first))
    QgsProject.instance().clear()
    other = lines(tmp_path, 'other')
    add(dock, DANGLES, other)
    second = tmp_path / 'second.qgz'
    assert QgsProject.instance().write(str(second))
    assert QgsProject.instance().read(str(first))
    assert queue_column(dock, 1) == ['roads', 'rails', 'stops']
    assert QgsProject.instance().read(str(second))
    assert queue_column(dock, 1) == ['other']
