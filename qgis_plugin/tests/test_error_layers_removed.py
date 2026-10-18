"""The user removes one of the layers of errors from the project: the errors
of the other layers stay as they were, and can still be shown on the map."""

from __future__ import annotations

import pytest
from qgis.core import QgsProject, QgsVectorLayer

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    click_run,
    error_layers,
    line_layer,
    node_text,
    result_nodes,
    select_error,
)

DANGLES = 'line/must-not-have-dangles'
OVERLAPS = 'line/must-not-overlap-with'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
# What each of the two layers of errors is about in the run below
GROUPS = {'Point errors': 'Must not have dangles', 'Line errors': 'overlap'}
KEPT = {'Point errors': 'Line errors', 'Line errors': 'Point errors'}


@pytest.fixture
def two_layers(dock: Dock) -> Dock:
    """Dangles, which are points, and an overlap, which is a line."""
    roads = line_layer(*CHAIN, name='roads')
    lakes = line_layer('LINESTRING(0 0, 10 0)', name='lakes')
    countries = line_layer('LINESTRING(5 0, 15 0)', name='countries')
    for layer in (roads, lakes, countries):
        QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, roads)
    add(dock, OVERLAPS, lakes, countries)
    click_run(dock)
    assert {layer.name() for layer in error_layers()} == set(GROUPS)
    return dock


def named(name: str) -> QgsVectorLayer:
    return next(layer for layer in error_layers() if layer.name() == name)


def remove(name: str) -> None:
    QgsProject.instance().removeMapLayer(named(name).id())


def group_texts(dock: Dock) -> list[str]:
    return [node_text(node) for node in result_nodes(dock, 0)]


@pytest.mark.parametrize('removed', GROUPS)
def test_the_tree_keeps_the_labels_of_the_other_groups(
    two_layers: Dock, removed: str
):
    kept = [
        text
        for text in group_texts(two_layers)
        if GROUPS[KEPT[removed]] in text
    ]
    assert len(kept) == 1
    remove(removed)
    assert group_texts(two_layers) == kept


def test_the_groups_that_are_left_stay_in_the_order_of_the_list(dock: Dock):
    # Alphabetically the groups would be alpha, lakes, zeta
    layers = {name: line_layer(*CHAIN, name=name) for name in ('zeta', 'alpha')}
    lakes = line_layer('LINESTRING(0 0, 10 0)', name='lakes')
    countries = line_layer('LINESTRING(5 0, 15 0)', name='countries')
    for layer in (*layers.values(), lakes, countries):
        QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, layers['zeta'])
    add(dock, OVERLAPS, lakes, countries)
    add(dock, DANGLES, layers['alpha'])
    click_run(dock)
    remove('Line errors')
    texts = group_texts(dock)
    assert [text.partition(':')[0] for text in texts] == ['zeta', 'alpha']


@pytest.mark.parametrize('removed', GROUPS)
def test_the_groups_of_the_removed_layer_leave_the_tree(
    two_layers: Dock, removed: str
):
    remove(removed)
    assert not any(GROUPS[removed] in text for text in group_texts(two_layers))


@pytest.mark.parametrize('removed', GROUPS)
def test_an_error_that_is_left_is_shown_on_the_map(
    two_layers: Dock, removed: str
):
    remove(removed)
    layer = named(KEPT[removed])
    select_error(two_layers, 0)
    row = two_layers.results.current_row()
    assert row is not None
    expected = sum(
        1
        for found in two_layers.results.rows
        if found.key == row.key and found.error == row.error
    )
    assert expected >= 1
    assert layer.featureCount() == expected
