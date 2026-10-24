"""Small things that a user meets within minutes: renames, layers with the
same name, the keyboard, a double click, a layer that goes away, many rules."""

from __future__ import annotations

import time
from pathlib import Path

from qgis.core import QgsProject, QgsRectangle, QgsVectorLayer
from qgis.gui import QgisInterface
from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtTest import QTest
from qgis.PyQt.QtWidgets import QApplication

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    both_engines,
    cell,
    checked,
    click_run,
    layer_nodes,
    line_layer,
    message_text,
    queue_column,
    result_nodes,
    rule_items,
    saved_layer,
    select_error,
    select_rules,
    set_checked,
    shown,
    wait_for,
)

pytestmark = both_engines

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)
FEATURE = 3
WIDE = QgsRectangle(-5000, -5000, 5000, 5000)


def lines(directory: Path, name: str = 'roads') -> QgsVectorLayer:
    directory.mkdir(parents=True, exist_ok=True)
    layer = saved_layer(directory, CHAIN, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


# Names


def test_renaming_a_layer_renames_its_node(dock: Dock, tmp_path: Path):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    roads.setName('main roads')
    assert layer_nodes(dock)[0].text(0) == 'main roads'


def test_two_layers_with_one_name_have_two_nodes(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path / 'a'))
    add(dock, DANGLES, lines(tmp_path / 'b'))
    assert len(layer_nodes(dock)) == 2


def test_two_layers_with_one_name_differ_in_their_tooltips(
    dock: Dock, tmp_path: Path
):
    add(dock, DANGLES, lines(tmp_path / 'a'))
    add(dock, DANGLES, lines(tmp_path / 'b'))
    first, second = layer_nodes(dock)
    assert first.toolTip(0) != second.toolTip(0)


def test_the_tooltip_of_a_geopackage_layer_has_its_file_and_its_name(
    dock: Dock, tmp_path: Path
):
    add(dock, DANGLES, lines(tmp_path / 'a', 'roads'))
    tooltip = layer_nodes(dock)[0].toolTip(0)
    assert str(tmp_path / 'a' / 'roads.gpkg') in tooltip
    assert 'roads' in tooltip


def test_the_tooltip_of_a_shapefile_layer_has_its_file(
    dock: Dock, tmp_path: Path
):
    folder = tmp_path / 'shapes'
    folder.mkdir()
    saved_layer(folder, CHAIN, 'rails', driver='ESRI Shapefile')
    layer = QgsVectorLayer(str(folder / 'rails.shp'), 'rails', 'ogr')
    QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, layer)
    assert str(folder / 'rails.shp') in layer_nodes(dock)[0].toolTip(0)


def test_the_boxes_work_with_two_layers_of_one_name(dock: Dock, tmp_path: Path):
    first = lines(tmp_path / 'a')
    second = lines(tmp_path / 'b')
    dock.input_box.setLayer(second)
    layer = dock.input_box.currentLayer()
    assert layer is not None
    assert layer.id() == second.id()
    add(dock, DANGLES, first)
    add(dock, DANGLES, second)
    assert {entry.layer for entry in dock.entries} == {first.id(), second.id()}


# Hover


def test_hovering_a_rule_shows_what_it_checks(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path))
    (item,) = rule_items(dock)
    rule = dock.entries[0].rule
    assert rule.description
    assert rule.description in item.toolTip(0)


# The keyboard


def press(widget: object, key: Qt.Key) -> None:
    QTest.keyClick(widget, key)
    QApplication.processEvents()


def test_delete_removes_the_selected_rules(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path / 'a', 'roads'))
    add(dock, DANGLES, lines(tmp_path / 'b', 'rails'))
    select_rules(dock, 0)
    press(dock.rules_tree, Qt.Key.Key_Delete)
    assert queue_column(dock, 1) == ['rails']


def test_backspace_removes_the_selected_rules(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path / 'a', 'roads'))
    add(dock, DANGLES, lines(tmp_path / 'b', 'rails'))
    select_rules(dock, 1)
    press(dock.rules_tree, Qt.Key.Key_Backspace)
    assert queue_column(dock, 1) == ['roads']


def test_delete_with_nothing_selected_removes_nothing(
    dock: Dock, tmp_path: Path
):
    add(dock, DANGLES, lines(tmp_path))
    dock.rules_tree.clearSelection()
    press(dock.rules_tree, Qt.Key.Key_Delete)
    assert len(dock.entries) == 1


def test_space_toggles_the_checkbox_of_a_rule(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path))
    select_rules(dock, 0)
    press(dock.rules_tree, Qt.Key.Key_Space)
    assert checked(dock) == [False]
    press(dock.rules_tree, Qt.Key.Key_Space)
    assert checked(dock) == [True]


def zoomed_run(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
) -> tuple[QgsVectorLayer, object]:
    roads = lines(tmp_path)
    canvas = qgis_iface.mapCanvas()
    canvas.setDestinationCrs(roads.crs())
    add(dock, DANGLES, roads)
    click_run(dock)
    return roads, canvas


def test_enter_on_an_error_zooms_to_it(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    _, canvas = zoomed_run(dock, tmp_path, qgis_iface)
    select_error(dock, 0)
    canvas.setExtent(WIDE)
    press(dock.results_view, Qt.Key.Key_Return)
    assert canvas.extent().width() < 1000


def test_the_enter_key_of_the_keypad_zooms_too(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    _, canvas = zoomed_run(dock, tmp_path, qgis_iface)
    select_error(dock, 0)
    canvas.setExtent(WIDE)
    press(dock.results_view, Qt.Key.Key_Enter)
    assert canvas.extent().width() < 1000


# Double click


def test_double_clicking_an_error_zooms_to_it(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    _, canvas = zoomed_run(dock, tmp_path, qgis_iface)
    canvas.setExtent(WIDE)
    dock.results_view.doubleClicked.emit(result_nodes(dock, 2)[0])
    assert canvas.extent().width() < 1000


def test_double_clicking_an_error_selects_its_features(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    roads, _ = zoomed_run(dock, tmp_path, qgis_iface)
    roads.removeSelection()
    dock.results_view.doubleClicked.emit(result_nodes(dock, 2)[0])
    fid = int(cell(dock, 0, FEATURE))
    assert set(roads.selectedFeatureIds()) == {fid}


def test_double_clicking_a_rule_selects_nothing(
    dock: Dock, tmp_path: Path, qgis_iface: QgisInterface
):
    roads, _ = zoomed_run(dock, tmp_path, qgis_iface)
    roads.removeSelection()
    dock.results_view.doubleClicked.emit(result_nodes(dock, 0)[0])
    assert roads.selectedFeatureIds() == []


# A layer that goes away during a run


def test_a_layer_removed_during_a_run_ends_the_run(dock: Dock, tmp_path: Path):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    dock.run_button.click()
    QgsProject.instance().removeMapLayer(roads.id())
    wait_for(lambda: dock.task is None)
    assert dock.run_button.text() in ('Run 1 rule', 'Run')


def test_a_layer_removed_during_a_run_is_marked_afterwards(
    dock: Dock, tmp_path: Path
):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    before = layer_nodes(dock)[0].icon(0).cacheKey()
    dock.run_button.click()
    QgsProject.instance().removeMapLayer(roads.id())
    wait_for(lambda: dock.task is None)
    # The tree may have been built again, so the node is looked up again
    (node,) = layer_nodes(dock)
    assert node.icon(0).cacheKey() != before
    assert node.toolTip(0)


def test_a_layer_removed_during_a_run_says_nothing_technical(
    dock: Dock, tmp_path: Path, bar: object
):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    dock.run_button.click()
    QgsProject.instance().removeMapLayer(roads.id())
    wait_for(lambda: dock.task is None)
    text = message_text(bar)
    for technical in ('Traceback', 'wrapped C/C++', 'NoneType'):
        assert technical not in text, technical


# Many rules


def fifty_rules(dock: Dock) -> float:
    """Adds 5 rules to each of 10 layers and returns the seconds that the
    clicks took, with the tree shown."""
    dock.resize(300, 900)
    dock.show()
    seconds = 0.0
    for number in range(10):
        layer = line_layer(*CHAIN, name=f'layer {number}')
        QgsProject.instance().addMapLayer(layer)
        dock.input_box.setLayer(layer)
        names = [
            dock.rule_box.itemData(index).name
            for index in range(dock.rule_box.count())
            if dock.rule_box.itemData(index).other is None
        ][:5]
        for name in names:
            index = [
                dock.rule_box.itemData(i).name
                for i in range(dock.rule_box.count())
            ].index(name)
            dock.rule_box.setCurrentIndex(index)
            started = time.monotonic()
            dock.add_button.click()
            QApplication.processEvents()
            seconds += time.monotonic() - started
    return seconds


def test_fifty_rules_are_added_and_shown_quickly(dock: Dock):
    assert fifty_rules(dock) < 1.0


def test_run_counts_fifty_rules(dock: Dock):
    fifty_rules(dock)
    assert len(dock.entries) == 50
    assert len(rule_items(dock)) == 50
    assert len(layer_nodes(dock)) == 10
    assert dock.run_button.text() == 'Run 50 rules'


# The count of each rule


def test_a_rule_that_ran_clean_shows_zero(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path / 'a', 'roads'))
    ring = saved_layer(tmp_path, RING, 'ring')
    QgsProject.instance().addMapLayer(ring)
    add(dock, DANGLES, ring)
    click_run(dock)
    assert queue_column(dock, 3) == ['2', '0']
    assert shown(dock) == 2


def test_a_rule_that_did_not_run_shows_nothing(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path / 'a', 'roads'))
    ring = saved_layer(tmp_path, RING, 'ring')
    QgsProject.instance().addMapLayer(ring)
    add(dock, DANGLES, ring)
    assert queue_column(dock, 3) == ['', '']
    set_checked(dock, 1, False)
    click_run(dock)
    assert queue_column(dock, 3) == ['2', '']
