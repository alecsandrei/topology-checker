"""The order in which the Tab key goes through the dock."""

from __future__ import annotations

from pathlib import Path

import pytest
from qgis.core import QgsProject, QgsVectorLayer
from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtWidgets import QApplication, QWidget

from topology_rules.panel.dock import Dock

from .helpers import add, line_layer, point_layer, saved_layer, select_rules

DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
# The widgets that the Tab key stops at, with their place in the dock
STOPS = (
    'input_box',
    'rule_box',
    'other_box',
    'add_button',
    'rules_tree',
    'remove_button',
    'clear_button',
    'import_button',
    'export_button',
    'options',
    'tolerance',
    'selected_only',
    'extent_only',
    'run_button',
)
# The stops that are inside the box of options, which opens to reach them
INSIDE = ('tolerance', 'selected_only', 'extent_only')


def owner_of(dock: Dock, name: str) -> QWidget:
    return dock.rules_page.options if name == 'options' else getattr(dock, name)


def stops(dock: Dock) -> list[str]:
    """The widgets of the dock that Tab reaches, in order, from the layer.

    The box of options is a stop by itself (its arrow); what it holds are
    stops of their own."""
    owners = {name: owner_of(dock, name) for name in STOPS}
    start: QWidget = dock.input_box
    found: list[str] = []
    widget = start
    for _ in range(500):
        if (
            widget.isVisibleTo(dock)
            and widget.isEnabled()
            and widget.focusPolicy() & Qt.FocusPolicy.TabFocus
        ):
            # What the box holds before the box, as it is their ancestor
            for name, owner in sorted(
                owners.items(), key=lambda item: item[0] == 'options'
            ):
                if widget is owner or owner.isAncestorOf(widget):
                    if not found or found[-1] != name:
                        found.append(name)
                    break
        widget = widget.nextInFocusChain()
        if widget is start:
            break
    return found


@pytest.fixture
def filled(dock: Dock, tmp_path: Path) -> Dock:
    """A shown dock with a rule in the list and selected, and a rule of two
    layers in the form."""
    dock.resize(300, 900)
    dock.show()
    QApplication.processEvents()
    roads = saved_layer(tmp_path, CHAIN, 'roads')
    stops_layer = point_layer('POINT(5 0)', name='stops')
    for layer in (roads, stops_layer):
        QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, roads)
    select_rules(dock, 0)
    dock.input_box.setLayer(stops_layer)
    dock.rule_box.setCurrentIndex(
        [
            dock.rule_box.itemData(i).name for i in range(dock.rule_box.count())
        ].index(COVERED)
    )
    QApplication.processEvents()
    return dock


@pytest.fixture
def opened(filled: Dock) -> Dock:
    """The same, with the box of options open."""
    filled.rules_page.options.setCollapsed(False)
    QApplication.processEvents()
    return filled


def position(order: list[str], name: str) -> int:
    assert name in order, f'{name} is not reached by Tab: {order}'
    return order.index(name)


def test_tab_starts_with_the_layer(filled: Dock):
    assert stops(filled)[0] == 'input_box'


def test_tab_goes_from_the_layer_to_the_rule_to_the_other_layer(filled: Dock):
    order = stops(filled)
    assert (
        position(order, 'input_box')
        < position(order, 'rule_box')
        < position(order, 'other_box')
        < position(order, 'add_button')
    )


def test_tab_goes_from_add_to_the_list(filled: Dock):
    order = stops(filled)
    assert position(order, 'add_button') < position(order, 'rules_tree')


def test_tab_goes_from_the_list_to_remove_and_clear(filled: Dock):
    order = stops(filled)
    first = min(
        position(order, 'remove_button'), position(order, 'clear_button')
    )
    assert position(order, 'rules_tree') < first


def test_tab_goes_from_remove_and_clear_to_import_and_export(filled: Dock):
    order = stops(filled)
    last = max(
        position(order, 'remove_button'), position(order, 'clear_button')
    )
    first = min(
        position(order, 'import_button'), position(order, 'export_button')
    )
    assert last < first


def test_tab_goes_from_import_and_export_to_the_options_to_run(opened: Dock):
    order = stops(opened)
    last = max(
        position(order, 'import_button'), position(order, 'export_button')
    )
    assert last < position(order, 'options') < position(order, 'run_button')


@pytest.mark.parametrize('name', INSIDE)
def test_tab_goes_from_the_options_into_the_box_to_run(opened: Dock, name: str):
    order = stops(opened)
    assert (
        position(order, 'options')
        < position(order, name)
        < position(order, 'run_button')
    )


def test_every_stop_is_reached_once(opened: Dock):
    order = stops(opened)
    assert sorted(order) == sorted(STOPS)
    assert len(order) == len(set(order))


def test_a_collapsed_box_is_a_stop_itself(filled: Dock):
    assert filled.rules_page.options.isCollapsed()
    order = stops(filled)
    last = max(
        position(order, 'import_button'), position(order, 'export_button')
    )
    assert last < position(order, 'options') < position(order, 'run_button')


@pytest.mark.parametrize('name', INSIDE)
def test_what_a_collapsed_box_holds_is_not_a_stop(filled: Dock, name: str):
    assert name not in stops(filled)


def test_a_collapsed_box_is_reached_once(filled: Dock):
    order = stops(filled)
    assert order.count('options') == 1
    assert len(order) == len(set(order))


def test_the_other_layer_is_skipped_when_it_is_hidden(
    dock: Dock, tmp_path: Path
):
    roads = saved_layer(tmp_path, CHAIN, 'roads')
    QgsProject.instance().addMapLayer(roads)
    add(dock, DANGLES, roads)
    dock.input_box.setLayer(roads)
    dock.resize(300, 900)
    dock.show()
    QApplication.processEvents()
    assert dock.other_box.isHidden()
    assert 'other_box' not in stops(dock)


def test_a_hidden_widget_is_never_a_stop(dock: Dock, tmp_path: Path):
    layer: QgsVectorLayer = line_layer(*CHAIN, name='lines')
    QgsProject.instance().addMapLayer(layer)
    dock.input_box.setLayer(layer)
    dock.resize(300, 900)
    dock.show()
    QApplication.processEvents()
    order = stops(dock)
    for name in order:
        assert owner_of(dock, name).isVisibleTo(dock), name
