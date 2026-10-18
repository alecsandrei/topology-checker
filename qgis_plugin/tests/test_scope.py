"""Checking only a part of a layer: the selected features, or what the map
shows."""

from __future__ import annotations

from pathlib import Path

import pytest
from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsCoordinateTransform,
    QgsProject,
    QgsRectangle,
    QgsVectorLayer,
)
from qgis.gui import QgisInterface
from qgis.PyQt.QtWidgets import QCheckBox

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    click_run,
    message_text,
    node_text,
    queue_column,
    result_nodes,
    result_rows,
    saved_layer,
    shown,
)

DANGLES = 'line/must-not-have-dangles'
# Two lines meeting at a node, 1 and 2, and a line far from them, 3
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
FAR = 'LINESTRING(0 50, 10 50)'
LONG = 'LINESTRING(0 0, 1000 0)'
AROUND_CHAIN = QgsRectangle(-1, -5, 21, 5)
AROUND_FAR = QgsRectangle(-1, 45, 11, 55)


def project_layer(
    tmp_path: Path, wkts: tuple[str, ...], name: str = 'roads'
) -> QgsVectorLayer:
    tmp_path.mkdir(parents=True, exist_ok=True)
    layer = saved_layer(tmp_path, wkts, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def look_at(
    iface: QgisInterface, area: QgsRectangle, layer: QgsVectorLayer
) -> None:
    """Puts the area of the layer in the CRS of the layer on the canvas."""
    canvas = iface.mapCanvas()
    canvas.setDestinationCrs(layer.crs())
    canvas.setExtent(area)


def features(dock: Dock) -> set[str]:
    return {row[3] for row in result_rows(dock)}


# The options


def test_the_scope_options_are_check_boxes(dock: Dock):
    assert isinstance(dock.selected_only, QCheckBox)
    assert isinstance(dock.extent_only, QCheckBox)
    assert dock.selected_only.text() == 'Selected features only'
    assert dock.extent_only.text() == 'Only the visible extent'


def test_the_scope_options_start_off(dock: Dock):
    assert not dock.selected_only.isChecked()
    assert not dock.extent_only.isChecked()


# The selected features


def test_without_the_option_the_whole_layer_is_checked(
    dock: Dock, tmp_path: Path
):
    layer = project_layer(tmp_path, (*CHAIN, FAR))
    layer.selectByIds([1, 2])
    add(dock, DANGLES, layer)
    click_run(dock)
    assert shown(dock) == 4


def test_selected_only_checks_only_the_selected_features(
    dock: Dock, tmp_path: Path
):
    layer = project_layer(tmp_path, (*CHAIN, FAR))
    layer.selectByIds([1, 2])
    add(dock, DANGLES, layer)
    dock.selected_only.setChecked(True)
    click_run(dock)
    assert shown(dock) == 2
    assert queue_column(dock, 3) == ['2']


def test_selected_only_keeps_the_ids_of_the_source(dock: Dock, tmp_path: Path):
    layer = project_layer(tmp_path, (*CHAIN, FAR))
    layer.selectByIds([3])
    add(dock, DANGLES, layer)
    dock.selected_only.setChecked(True)
    click_run(dock)
    assert features(dock) == {'3'}


def test_the_option_counts_when_the_run_starts_not_when_a_rule_is_added(
    dock: Dock, tmp_path: Path
):
    layer = project_layer(tmp_path, (*CHAIN, FAR))
    layer.selectByIds([1, 2])
    dock.selected_only.setChecked(True)
    add(dock, DANGLES, layer)
    dock.selected_only.setChecked(False)
    click_run(dock)
    assert shown(dock) == 4


def test_the_selection_of_the_moment_of_the_run_is_used(
    dock: Dock, tmp_path: Path
):
    layer = project_layer(tmp_path, (*CHAIN, FAR))
    add(dock, DANGLES, layer)
    dock.selected_only.setChecked(True)
    layer.selectByIds([1, 2])
    click_run(dock)
    assert shown(dock) == 2
    layer.selectByIds([3])
    click_run(dock)
    assert features(dock) == {'3'}


def test_a_layer_without_a_selection_contributes_nothing(
    dock: Dock, tmp_path: Path
):
    chosen = project_layer(tmp_path / 'a', CHAIN, 'chosen')
    other = project_layer(tmp_path / 'b', CHAIN, 'other')
    chosen.selectByIds([1, 2])
    add(dock, DANGLES, chosen)
    add(dock, DANGLES, other)
    dock.selected_only.setChecked(True)
    click_run(dock)
    assert queue_column(dock, 3) == ['2', '0']
    texts = [node_text(node) for node in result_nodes(dock, 0)]
    assert texts == ['chosen: Must not have dangles (2)']


def test_the_message_says_which_layer_had_no_selection(
    dock: Dock, tmp_path: Path, bar: object
):
    chosen = project_layer(tmp_path / 'a', CHAIN, 'chosen')
    other = project_layer(tmp_path / 'b', CHAIN, 'other')
    chosen.selectByIds([1, 2])
    add(dock, DANGLES, chosen)
    add(dock, DANGLES, other)
    dock.selected_only.setChecked(True)
    click_run(dock)
    text = message_text(bar)
    assert 'other' in text
    assert 'selected' in text.lower()


def test_no_selection_at_all_says_so(dock: Dock, tmp_path: Path, bar: object):
    layer = project_layer(tmp_path, CHAIN, 'roads')
    add(dock, DANGLES, layer)
    dock.selected_only.setChecked(True)
    click_run(dock)
    assert shown(dock) == 0
    text = message_text(bar)
    assert 'roads' in text
    assert 'selected' in text.lower()


# The visible extent


def test_the_visible_extent_limits_the_features(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    layer = project_layer(tmp_path, (*CHAIN, FAR))
    look_at(qgis_iface, AROUND_CHAIN, layer)
    add(dock, DANGLES, layer)
    dock.extent_only.setChecked(True)
    click_run(dock)
    assert shown(dock) == 2


def test_the_visible_extent_keeps_the_ids_of_the_source(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    layer = project_layer(tmp_path, (*CHAIN, FAR))
    look_at(qgis_iface, AROUND_FAR, layer)
    add(dock, DANGLES, layer)
    dock.extent_only.setChecked(True)
    click_run(dock)
    assert features(dock) == {'3'}


def test_a_feature_that_crosses_the_extent_is_checked_whole(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    layer = project_layer(tmp_path, (LONG,))
    look_at(qgis_iface, QgsRectangle(-1, -5, 21, 5), layer)
    add(dock, DANGLES, layer)
    dock.extent_only.setChecked(True)
    click_run(dock)
    # Both ends of the line, one of them a kilometre out of the view
    assert shown(dock) == 2


def test_the_extent_of_another_crs_is_reprojected(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    layer = project_layer(tmp_path, (*CHAIN, FAR))
    geographic = QgsCoordinateReferenceSystem('EPSG:4326')
    transform = QgsCoordinateTransform(
        layer.crs(), geographic, QgsProject.instance()
    )
    canvas = qgis_iface.mapCanvas()
    canvas.setDestinationCrs(geographic)
    canvas.setExtent(transform.transformBoundingBox(AROUND_CHAIN))
    add(dock, DANGLES, layer)
    dock.extent_only.setChecked(True)
    click_run(dock)
    assert shown(dock) == 2
    assert features(dock) <= {'1', '2'}


def test_an_extent_without_features_checks_nothing(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    layer = project_layer(tmp_path, CHAIN)
    look_at(qgis_iface, QgsRectangle(5000, 5000, 5100, 5100), layer)
    add(dock, DANGLES, layer)
    dock.extent_only.setChecked(True)
    click_run(dock)
    assert shown(dock) == 0


def test_the_extent_of_the_moment_of_the_run_is_used(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    layer = project_layer(tmp_path, (*CHAIN, FAR))
    add(dock, DANGLES, layer)
    dock.extent_only.setChecked(True)
    look_at(qgis_iface, AROUND_CHAIN, layer)
    click_run(dock)
    assert features(dock) <= {'1', '2'}
    look_at(qgis_iface, AROUND_FAR, layer)
    click_run(dock)
    assert features(dock) == {'3'}


# Both


def test_both_options_check_the_selected_features_in_the_view(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    layer = project_layer(tmp_path, (*CHAIN, FAR))
    layer.selectByIds([1, 3])
    look_at(qgis_iface, AROUND_CHAIN, layer)
    add(dock, DANGLES, layer)
    dock.selected_only.setChecked(True)
    dock.extent_only.setChecked(True)
    click_run(dock)
    # The feature 1 alone: it does not meet the feature 2, which is not taken
    assert features(dock) == {'1'}
    assert shown(dock) == 2


# The project


@pytest.mark.parametrize('name', ['selected_only', 'extent_only'])
def test_an_option_is_saved_with_the_project(
    dock: Dock, tmp_path: Path, name: str
):
    layer = project_layer(tmp_path, CHAIN)
    add(dock, DANGLES, layer)
    getattr(dock, name).setChecked(True)
    path = tmp_path / 'project.qgz'
    assert QgsProject.instance().write(str(path))
    QgsProject.instance().clear()
    assert not getattr(dock, name).isChecked()
    assert QgsProject.instance().read(str(path))
    assert getattr(dock, name).isChecked()


def test_the_option_that_was_off_stays_off_after_reading(
    dock: Dock, tmp_path: Path
):
    layer = project_layer(tmp_path, CHAIN)
    add(dock, DANGLES, layer)
    dock.selected_only.setChecked(True)
    path = tmp_path / 'project.qgz'
    assert QgsProject.instance().write(str(path))
    QgsProject.instance().clear()
    assert QgsProject.instance().read(str(path))
    assert not dock.extent_only.isChecked()


def test_a_new_project_has_both_options_off(dock: Dock):
    dock.selected_only.setChecked(True)
    dock.extent_only.setChecked(True)
    QgsProject.instance().clear()
    assert not dock.selected_only.isChecked()
    assert not dock.extent_only.isChecked()
