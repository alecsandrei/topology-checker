"""The helpers that read the trees of the dock, on trees of their own."""

from __future__ import annotations

import types
import typing as t

import pytest
from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtGui import QStandardItem, QStandardItemModel
from qgis.PyQt.QtWidgets import QTreeView, QTreeWidget, QTreeWidgetItem

from .helpers import (
    cell,
    checked,
    queue_column,
    result_rows,
    select_error,
    select_result,
    select_rules,
    set_checked,
    shown,
)


@pytest.fixture
def stub() -> t.Any:
    """A dock with a list of two layers and results of two rules."""
    tree = QTreeWidget()
    tree.setColumnCount(2)
    roads = QTreeWidgetItem(tree, ['roads', ''])
    stops = QTreeWidgetItem(tree, ['stops', ''])
    for parent, text, count in (
        (roads, 'Must not have dangles', '2'),
        (stops, 'Must be covered by lines with roads', '1'),
    ):
        item = QTreeWidgetItem(parent, [text, count])
        item.setCheckState(0, Qt.CheckState.Checked)

    model = QStandardItemModel()

    def node(parent: QStandardItem, text: str) -> QStandardItem:
        item = QStandardItem(text)
        parent.appendRow(item)
        return item

    root = model.invisibleRootItem()
    dangles = node(root, 'roads: Must not have dangles (2)')
    kind = node(dangles, 'Dangles (2)')
    node(kind, 'roads 1')
    node(kind, 'roads 2')
    covered = node(root, 'stops: Must be covered by lines with roads (1)')
    kind = node(covered, 'Not covered (1)')
    node(kind, 'lakes 6 · countries 168')
    view = QTreeView()
    view.setModel(model)
    return types.SimpleNamespace(rules_tree=tree, results_view=view)


def test_the_rules_are_read_in_the_order_of_the_tree(stub: t.Any):
    assert queue_column(stub, 0) == [
        'Must not have dangles',
        'Must be covered by lines',
    ]
    assert queue_column(stub, 1) == ['roads', 'stops']
    assert queue_column(stub, 2) == ['', 'roads']
    assert queue_column(stub, 3) == ['2', '1']


def test_the_checkboxes_are_read_and_set(stub: t.Any):
    assert checked(stub) == [True, True]
    set_checked(stub, 1, False)
    assert checked(stub) == [True, False]


def test_rules_are_selected_by_their_place(stub: t.Any):
    select_rules(stub, 1)
    assert [item.text(0) for item in stub.rules_tree.selectedItems()] == [
        'Must be covered by lines with roads'
    ]


def test_the_errors_are_counted_and_read(stub: t.Any):
    assert shown(stub) == 3
    assert result_rows(stub)[0] == [
        'Must not have dangles',
        'Dangles',
        'roads',
        '1',
        '',
        '',
    ]


def test_an_error_of_two_layers_is_split(stub: t.Any):
    assert result_rows(stub)[2] == [
        'Must be covered by lines with roads',
        'Not covered',
        'lakes',
        '6',
        'countries',
        '168',
    ]
    assert cell(stub, 2, 4) == 'countries'


def test_an_error_is_selected(stub: t.Any):
    select_error(stub, 1)
    assert stub.results_view.currentIndex().data() == 'roads 2'


def test_a_node_is_selected_by_a_part_of_its_text(stub: t.Any):
    index = select_result(stub, 'Dangles')
    assert index.data() == 'Dangles (2)'
    assert select_result(stub, 'stops: Must').data().startswith('stops')
