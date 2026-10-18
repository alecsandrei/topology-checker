"""The list of rules as a tree: layers that hold their rules."""

from __future__ import annotations

import re
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsProject, QgsVectorLayer
from qgis.PyQt.QtCore import QPoint, QRect, Qt
from qgis.PyQt.QtWidgets import QApplication, QTreeWidget

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    click_run,
    layer_nodes,
    line_layer,
    point_layer,
    queue_column,
    rule_items,
    saved_layer,
    select_rules,
)
from .test_wrapping import SLACK, room

DANGLES = 'line/must-not-have-dangles'
OVERLAPS = 'line/must-not-overlap'
COVERED = 'point/must-be-covered-by-lines'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


def lines(tmp_path: Path, name: str = 'roads') -> QgsVectorLayer:
    layer = saved_layer(tmp_path, CHAIN, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def stops() -> QgsVectorLayer:
    layer = point_layer('POINT(5 0)', 'POINT(5 5)', name='stops')
    QgsProject.instance().addMapLayer(layer)
    return layer


def test_the_list_is_a_tree(dock: Dock):
    assert isinstance(dock.rules_tree, QTreeWidget)


def test_the_placeholder_shows_only_for_an_empty_list(
    dock: Dock, tmp_path: Path
):
    assert not dock.rules_placeholder.isHidden()
    add(dock, DANGLES, lines(tmp_path))
    assert dock.rules_placeholder.isHidden()
    dock.clear_rules()
    assert not dock.rules_placeholder.isHidden()


def test_a_layer_is_a_node_with_its_name_and_icon(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path))
    (node,) = layer_nodes(dock)
    assert node.text(0) == 'roads'
    assert not node.icon(0).isNull()


def test_the_rules_of_a_layer_are_its_children(dock: Dock, tmp_path: Path):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    add(dock, OVERLAPS, roads)
    (node,) = layer_nodes(dock)
    assert [node.child(row).text(0) for row in range(node.childCount())] == [
        'Must not have dangles',
        'Must not overlap',
    ]


def test_layers_keep_the_order_in_which_they_were_added(
    dock: Dock, tmp_path: Path
):
    add(dock, DANGLES, lines(tmp_path, 'roads'))
    add(dock, DANGLES, lines(tmp_path, 'rails'))
    assert [node.text(0) for node in layer_nodes(dock)] == ['roads', 'rails']


def test_a_rule_of_two_layers_names_the_other_layer(dock: Dock, tmp_path: Path):
    add(dock, COVERED, stops(), lines(tmp_path))
    (item,) = rule_items(dock)
    assert item.text(0) == 'Must be covered by lines with roads'


def test_a_rule_has_a_checkbox(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path))
    (item,) = rule_items(dock)
    assert item.flags() & Qt.ItemFlag.ItemIsUserCheckable


def test_the_second_column_counts_the_errors(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path))
    assert queue_column(dock, 3) == ['']
    click_run(dock)
    assert queue_column(dock, 3) == ['2']


def test_a_missing_layer_puts_the_warning_on_its_node(
    dock: Dock, tmp_path: Path
):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    (node,) = layer_nodes(dock)
    before = node.icon(0).cacheKey()
    QgsProject.instance().removeMapLayer(roads.id())
    assert not node.icon(0).isNull()
    assert node.icon(0).cacheKey() != before
    assert node.toolTip(0)


def test_a_missing_layer_does_not_paint_the_rules_red(
    dock: Dock, tmp_path: Path
):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    QgsProject.instance().removeMapLayer(roads.id())
    (item,) = rule_items(dock)
    assert item.foreground(0).style() == Qt.BrushStyle.NoBrush


def test_removing_a_rule_keeps_its_layer_when_others_remain(
    dock: Dock, tmp_path: Path
):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    add(dock, OVERLAPS, roads)
    select_rules(dock, 0)
    dock.remove_button.click()
    assert queue_column(dock, 0) == ['Must not overlap']
    assert [node.text(0) for node in layer_nodes(dock)] == ['roads']


def test_removing_the_last_rule_of_a_layer_removes_the_node(
    dock: Dock, tmp_path: Path
):
    add(dock, DANGLES, lines(tmp_path, 'roads'))
    add(dock, DANGLES, lines(tmp_path, 'rails'))
    select_rules(dock, 0)
    dock.remove_button.click()
    assert [node.text(0) for node in layer_nodes(dock)] == ['rails']


def test_several_rules_can_be_removed_at_once(dock: Dock, tmp_path: Path):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    add(dock, OVERLAPS, roads)
    add(dock, DANGLES, lines(tmp_path, 'rails'))
    select_rules(dock, 0, 2)
    dock.remove_button.click()
    assert queue_column(dock, 0) == ['Must not overlap']


def test_a_long_name_does_not_need_a_horizontal_scroll_bar(
    dock: Dock, tmp_path: Path
):
    long = 'a layer with a very long name that does not fit in a narrow dock'
    layer = line_layer(*CHAIN, name=long)
    QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, layer)
    add(dock, OVERLAPS, layer)
    dock.resize(300, 700)
    dock.show()
    QApplication.processEvents()
    assert not dock.rules_tree.horizontalScrollBar().isVisible()


def test_double_clicking_a_rule_shows_its_errors(dock: Dock, tmp_path: Path):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    add(dock, OVERLAPS, roads)
    click_run(dock)
    dock.tabs.setCurrentIndex(0)
    first, _ = rule_items(dock)
    dock.rules_tree.itemDoubleClicked.emit(first, 0)
    assert dock.tabs.currentIndex() == 1
    current = dock.results_view.currentIndex()
    assert 'Must not have dangles' in current.data()


def test_double_clicking_a_layer_does_not_change_the_tab(
    dock: Dock, tmp_path: Path
):
    add(dock, DANGLES, lines(tmp_path))
    click_run(dock)
    dock.tabs.setCurrentIndex(0)
    (node,) = layer_nodes(dock)
    dock.rules_tree.itemDoubleClicked.emit(node, 0)
    assert dock.tabs.currentIndex() == 0


# The empty state: the placeholder sits inside the empty widget


def shown_dock(dock: Dock) -> Dock:
    dock.resize(300, 900)
    dock.show()
    QApplication.processEvents()
    return dock


def area(widget: t.Any, dock: Dock) -> QRect:
    """The rectangle of a widget, in the coordinates of the dock."""
    return QRect(widget.mapTo(dock, QPoint(0, 0)), widget.size())


def test_the_empty_list_is_visible_with_its_placeholder(dock: Dock):
    shown_dock(dock)
    assert dock.rules_tree.isVisible()
    assert dock.rules_placeholder.isVisible()


def test_the_placeholder_of_the_list_is_inside_the_list(dock: Dock):
    shown_dock(dock)
    viewport = area(dock.rules_tree.viewport(), dock)
    assert viewport.contains(area(dock.rules_placeholder, dock))


def test_the_empty_results_are_visible_with_their_placeholder(dock: Dock):
    shown_dock(dock)
    dock.tabs.setCurrentIndex(1)
    QApplication.processEvents()
    assert dock.results_view.isVisible()
    assert dock.results_placeholder.isVisible()


def test_the_placeholder_of_the_results_is_inside_the_results(dock: Dock):
    shown_dock(dock)
    dock.tabs.setCurrentIndex(1)
    QApplication.processEvents()
    viewport = area(dock.results_view.viewport(), dock)
    assert viewport.contains(area(dock.results_placeholder, dock))


def test_the_first_rule_does_not_move_the_list(dock: Dock, tmp_path: Path):
    shown_dock(dock)
    before = area(dock.rules_tree, dock)
    add(dock, DANGLES, lines(tmp_path))
    QApplication.processEvents()
    assert area(dock.rules_tree, dock) == before
    assert dock.rules_placeholder.isHidden()


# At the width of a narrow dock, with the room that the style leaves


@pytest.mark.usefixtures('real_font')
def test_a_short_rule_name_is_not_elided_at_300_px(dock: Dock, tmp_path: Path):
    shown_dock(dock)
    add(dock, DANGLES, lines(tmp_path))
    QApplication.processEvents()
    (item,) = rule_items(dock)
    assert item.text(0) == 'Must not have dangles'
    tree = dock.rules_tree
    width = tree.fontMetrics().horizontalAdvance(item.text(0))
    # The room that the style leaves for the text, with a few pixels to spare:
    # the checkbox, the icon and the margins differ from one style to another
    assert width <= room(tree, tree.indexFromItem(item, 0), SLACK)


def test_the_second_column_holds_only_numbers(dock: Dock, tmp_path: Path):
    roads = lines(tmp_path)
    add(dock, DANGLES, roads)
    add(dock, OVERLAPS, roads)
    click_run(dock)
    for node in layer_nodes(dock):
        assert re.fullmatch(r'\d*', node.text(1)), node.text(0)
    for item in rule_items(dock):
        assert re.fullmatch(r'\d*', item.text(1)), item.text(0)


@pytest.mark.usefixtures('real_font')
def test_the_second_column_is_narrow(dock: Dock, tmp_path: Path):
    shown_dock(dock)
    add(dock, DANGLES, lines(tmp_path))
    click_run(dock)
    assert dock.rules_tree.columnWidth(1) <= 60
