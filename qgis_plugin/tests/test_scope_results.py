"""The results of a run that checked only a part of the layers work as those
of a run of the whole layers: the names of the layers, the buttons that select
the features, the invalid features and the errors that were ignored."""

from __future__ import annotations

from pathlib import Path

import pytest
from qgis.core import QgsProject, QgsVectorLayer
from qgis.gui import QgisInterface

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    both_engines,
    cell,
    click_run,
    layer_nodes,
    polygon_layer,
    result_nodes,
    result_rows,
    select_error,
    shown,
)
from .test_invalid import BOWTIE, OVERLAPPING, OVERLAPS, SQUARE
from .test_scope import CHAIN, DANGLES, FAR, look_at, project_layer

pytestmark = both_engines

NAME = 'Main roads'
HOW = ['selected', 'extent']


def limit(
    dock: Dock, iface: QgisInterface, layer: QgsVectorLayer, how: str
) -> None:
    """Makes the next run check a part of the layer: its selected features,
    or what the map shows. The part is the whole layer here."""
    if how == 'selected':
        layer.selectAll()
        dock.selected_only.setChecked(True)
    else:
        look_at(iface, layer.extent().buffered(10), layer)
        dock.extent_only.setChecked(True)


@pytest.fixture
def roads(dock: Dock, tmp_path: Path) -> QgsVectorLayer:
    """A layer of two lines that meet, and one far from them, in the dock."""
    layer = project_layer(tmp_path, (*CHAIN, FAR), NAME)
    add(dock, DANGLES, layer)
    return layer


# The names of the layers


@pytest.mark.parametrize('how', HOW)
def test_a_limited_run_names_the_layers_in_the_results(
    dock: Dock,
    qgis_iface: QgisInterface,
    roads: QgsVectorLayer,
    how: str,
):
    limit(dock, qgis_iface, roads, how)
    click_run(dock)
    assert {row[2] for row in result_rows(dock)} == {NAME}


def test_a_whole_run_names_the_layers_in_the_results(
    dock: Dock, roads: QgsVectorLayer
):
    click_run(dock)
    assert {row[2] for row in result_rows(dock)} == {NAME}


# Selecting the features of an error


@pytest.mark.parametrize('how', HOW)
def test_select_features_selects_the_source_features_of_a_limited_run(
    dock: Dock,
    qgis_iface: QgisInterface,
    roads: QgsVectorLayer,
    how: str,
):
    limit(dock, qgis_iface, roads, how)
    click_run(dock)
    roads.removeSelection()
    select_error(dock, 0)
    dock.select_button.click()
    assert roads.selectedFeatureIds() == [int(cell(dock, 0, 3))]


@pytest.mark.parametrize('how', HOW)
def test_a_double_click_selects_the_source_features_of_a_limited_run(
    dock: Dock,
    qgis_iface: QgisInterface,
    roads: QgsVectorLayer,
    how: str,
):
    limit(dock, qgis_iface, roads, how)
    click_run(dock)
    roads.removeSelection()
    dock.results_view.doubleClicked.emit(result_nodes(dock, 2)[0])
    assert roads.selectedFeatureIds() == [int(cell(dock, 0, 3))]


# The invalid features


@pytest.mark.parametrize('how', HOW)
def test_a_limited_run_counts_the_invalid_features_of_a_layer(
    dock: Dock, qgis_iface: QgisInterface, how: str
):
    layer = polygon_layer(SQUARE, OVERLAPPING, BOWTIE, name='parcels')
    QgsProject.instance().addMapLayer(layer)
    add(dock, OVERLAPS, layer)
    limit(dock, qgis_iface, layer, how)
    click_run(dock)
    (node,) = layer_nodes(dock)
    assert node.text(0) == 'parcels · 1 invalid'


# The errors that were ignored


@pytest.mark.parametrize('how', HOW)
def test_an_error_ignored_in_a_whole_run_stays_ignored_in_a_limited_run(
    dock: Dock,
    qgis_iface: QgisInterface,
    roads: QgsVectorLayer,
    how: str,
):
    click_run(dock)
    whole = shown(dock)
    select_error(dock, 0)
    dock.results.ignore_action.trigger()
    assert shown(dock) == whole - 1
    limit(dock, qgis_iface, roads, how)
    click_run(dock)
    assert shown(dock) == whole - 1
