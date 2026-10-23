"""The results of a run: a tree of rules, types of error and errors."""

from __future__ import annotations

import re
import time
import typing as t
from pathlib import Path

import pytest
from qgis.core import (
    QgsLayerTreeLayer,
    QgsProject,
    QgsRectangle,
    QgsVectorLayer,
)
from qgis.gui import QgisInterface, QgsHighlight
from qgis.PyQt.QtCore import QModelIndex, Qt
from qgis.PyQt.QtWidgets import QApplication, QTreeView

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    both_engines,
    cell,
    click_run,
    line_layer,
    node_text,
    point_layer,
    polygon_layer,
    result_nodes,
    result_rows,
    saved_layer,
    select_error,
    select_result,
    shown,
)

pytestmark = both_engines

DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
OVERLAPS = 'line/must-not-overlap-with'
FEATURE, OTHER_FEATURE = 3, 5
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)


def project_lines(
    tmp_path: Path, wkts: tuple[str, ...], name: str = 'roads'
) -> QgsVectorLayer:
    layer = saved_layer(tmp_path, wkts, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def numbered_lines(count: int, shift: float, name: str) -> QgsVectorLayer:
    """Lines at y = 0, 1, ... that overlap the lines of another such layer
    that is shifted along x."""
    layer = line_layer(
        *[f'LINESTRING({shift} {y}, {shift + 10} {y})' for y in range(count)],
        name=name,
    )
    QgsProject.instance().addMapLayer(layer)
    return layer


@pytest.fixture
def overlaps(dock: Dock) -> Dock:
    """Twelve overlaps between two layers, each of feature n and feature n."""
    first = numbered_lines(12, 0, 'first')
    second = numbered_lines(12, 5, 'second')
    add(dock, OVERLAPS, first, second)
    click_run(dock)
    return dock


@pytest.fixture
def mixed(dock: Dock, tmp_path: Path) -> Dock:
    """Two errors of one rule, one of another, and a rule without errors."""
    roads = project_lines(tmp_path, CHAIN, 'roads')
    rails = project_lines(tmp_path, RING, 'rails')
    stops = point_layer('POINT(5 0)', 'POINT(5 5)', name='stops')
    QgsProject.instance().addMapLayer(stops)
    add(dock, DANGLES, roads)
    add(dock, DANGLES, rails)
    add(dock, COVERED, stops, roads)
    click_run(dock)
    return dock


def error_layer(geometry: str) -> QgsVectorLayer:
    """The layer of the project that holds the errors of a geometry."""
    names = {'point': 0, 'line': 1, 'polygon': 2}
    for layer in QgsProject.instance().mapLayers().values():
        if (
            isinstance(layer, QgsVectorLayer)
            and layer.fields().indexOf('violation_id') >= 0
            and int(layer.geometryType()) == names[geometry]
        ):
            return layer
    raise AssertionError(f'no layer of {geometry} errors')


def count_of(text: str) -> int:
    match = re.search(r'\((\d+)\)$', text)
    assert match, text
    return int(match.group(1))


# The tree


def test_the_results_are_a_tree_view(dock: Dock):
    assert isinstance(dock.results_view, QTreeView)


def test_the_placeholder_is_visible_before_the_first_run(dock: Dock):
    assert not dock.results_placeholder.isHidden()


def test_the_placeholder_goes_with_the_first_results(mixed: Dock):
    assert mixed.results_placeholder.isHidden()


def test_a_rule_with_errors_is_a_node_with_its_layer_and_count(mixed: Dock):
    texts = [node_text(node) for node in result_nodes(mixed, 0)]
    assert texts == [
        'roads: Must not have dangles (2)',
        'stops: Must be covered by lines with roads (1)',
    ]


def test_a_type_of_error_is_a_node_with_its_plural_and_count(mixed: Dock):
    dangles = next(
        node
        for node in result_nodes(mixed, 1)
        if 'Must not have dangles' in node.parent().data()
    )
    assert node_text(dangles) == 'Dangles (2)'


def test_the_same_rule_on_two_layers_has_two_nodes(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_lines(tmp_path, CHAIN, 'roads'))
    add(
        dock,
        DANGLES,
        project_lines(tmp_path, (*CHAIN, 'LINESTRING(0 50, 10 50)'), 'tracks'),
    )
    click_run(dock)
    texts = sorted(node_text(node) for node in result_nodes(dock, 0))
    assert texts == [
        'roads: Must not have dangles (2)',
        'tracks: Must not have dangles (4)',
    ]


def test_a_two_layer_rule_names_its_other_layer_in_its_node(overlaps: Dock):
    (node,) = result_nodes(overlaps, 0)
    assert node_text(node) == 'first: Must not overlap with second (12)'


def test_the_counts_of_the_levels_agree(mixed: Dock):
    for depth in (0, 1):
        for node in result_nodes(mixed, depth):
            model = mixed.results_view.model()
            below = model.rowCount(node)
            if depth == 1:
                assert count_of(node_text(node)) == below
            else:
                assert count_of(node_text(node)) == sum(
                    count_of(node_text(model.index(row, 0, node)))
                    for row in range(below)
                )


def test_the_errors_are_the_leaves(mixed: Dock):
    assert shown(mixed) == 3
    model = mixed.results_view.model()
    for leaf in result_nodes(mixed, 2):
        assert model.rowCount(leaf) == 0


def test_the_results_are_a_single_column(overlaps: Dock):
    assert overlaps.results_view.model().columnCount() == 1


def test_the_type_of_an_overlap_is_a_plural(overlaps: Dock):
    (node,) = result_nodes(overlaps, 1)
    assert node_text(node) == 'Overlaps (12)'


def test_the_features_of_an_error_name_the_layer_and_the_feature(
    mixed: Dock,
):
    leaf = next(
        node
        for node in result_nodes(mixed, 2)
        if 'Must not have dangles' in node.parent().parent().data()
    )
    assert re.fullmatch(r'roads \d+', node_text(leaf))


def test_the_features_of_two_layers_are_joined_by_a_dot(overlaps: Dock):
    leaf = result_nodes(overlaps, 2)[0]
    assert re.fullmatch(r'first \d+ · second \d+', node_text(leaf))


def ring(dx: int) -> tuple[str, ...]:
    """Four rectangles around a hole: one gap, and the outer boundary."""
    x = dx
    return (
        f'POLYGON(({x} 0, {x + 6} 0, {x + 6} 2, {x} 2, {x} 0))',
        f'POLYGON(({x} 4, {x + 6} 4, {x + 6} 6, {x} 6, {x} 4))',
        f'POLYGON(({x} 2, {x + 2} 2, {x + 2} 4, {x} 4, {x} 2))',
        f'POLYGON(({x + 4} 2, {x + 6} 2, {x + 6} 4, {x + 4} 4, {x + 4} 2))',
    )


GAPS = 'polygon/must-not-have-gaps'


def parcels(name: str, *rings: tuple[str, ...]) -> QgsVectorLayer:
    layer = polygon_layer(*[wkt for ring in rings for wkt in ring], name=name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def gaps_dock(dock: Dock) -> Dock:
    add(dock, GAPS, parcels('parcels', ring(0)))
    click_run(dock)
    return dock


def test_errors_without_features_are_still_named(dock: Dock):
    gaps_dock(dock)
    texts = [node_text(leaf) for leaf in result_nodes(dock, 2)]
    assert len(texts) == 2
    assert all(text.strip() for text in texts)
    assert sorted(re.sub(r' \d+$', '', text) for text in texts) == [
        'Gap',
        'Outer boundary',
    ]


def test_every_error_without_features_ends_in_a_number(dock: Dock):
    gaps_dock(dock)
    for leaf in result_nodes(dock, 2):
        assert re.fullmatch(r'(Gap|Outer boundary) \d+', node_text(leaf))


def test_the_types_of_the_gaps_are_plurals(dock: Dock):
    gaps_dock(dock)
    texts = sorted(node_text(node) for node in result_nodes(dock, 1))
    assert texts == ['Gaps (1)', 'Outer boundaries (1)']


def test_each_feature_of_an_overlap_is_named(overlaps: Dock):
    rows = result_rows(overlaps)
    assert len(rows) == 12
    for row in rows:
        assert row[2] == 'first'
        assert row[4] == 'second'
        assert row[FEATURE] == row[OTHER_FEATURE]


def test_there_is_no_cap_on_the_errors(dock: Dock, tmp_path: Path):
    wkts = tuple(f'LINESTRING({3 * i} 0, {3 * i + 1} 0)' for i in range(3000))
    add(dock, DANGLES, project_lines(tmp_path, wkts))
    click_run(dock)
    assert shown(dock) == 6000


# The map follows the selection in the tree


def visible(geometry: str) -> int:
    return error_layer(geometry).featureCount()


def test_all_the_errors_show_on_the_map_by_default(mixed: Dock):
    assert visible('point') == 3


def test_selecting_a_rule_shows_only_its_errors_on_the_map(mixed: Dock):
    select_result(mixed, 'Must not have dangles')
    assert visible('point') == 2
    select_result(mixed, 'Must be covered by lines')
    assert visible('point') == 1


def test_selecting_a_type_of_error_shows_only_those_errors(mixed: Dock):
    select_result(mixed, 'Dangles')
    assert visible('point') == 2


def test_selecting_nothing_shows_all_the_errors_again(mixed: Dock):
    select_result(mixed, 'Must not have dangles')
    mixed.results_view.setCurrentIndex(QModelIndex())
    mixed.results_view.clearSelection()
    assert visible('point') == 3


def test_a_new_run_shows_all_the_errors(mixed: Dock):
    select_result(mixed, 'Must not have dangles')
    click_run(mixed)
    assert visible('point') == 3


def points_and_lines(dock: Dock, tmp_path: Path) -> None:
    add(dock, DANGLES, project_lines(tmp_path, CHAIN))
    first = numbered_lines(3, 0, 'first')
    second = numbered_lines(3, 5, 'second')
    add(dock, OVERLAPS, first, second)
    click_run(dock)


def test_a_rule_hides_the_errors_of_the_other_layers_of_errors(
    dock: Dock, tmp_path: Path
):
    points_and_lines(dock, tmp_path)
    select_result(dock, 'Must not have dangles')
    assert visible('point') == 2
    assert visible('line') == 0


def test_the_other_rule_hides_the_points_instead(dock: Dock, tmp_path: Path):
    points_and_lines(dock, tmp_path)
    select_result(dock, 'Must not overlap with')
    assert visible('point') == 0
    assert visible('line') == 3


def test_a_type_of_error_hides_the_other_layers_of_errors(
    dock: Dock, tmp_path: Path
):
    points_and_lines(dock, tmp_path)
    select_result(dock, 'Overlaps')
    assert visible('point') == 0
    assert visible('line') == 3


def test_a_rerun_clears_the_filter_of_every_layer(dock: Dock, tmp_path: Path):
    points_and_lines(dock, tmp_path)
    select_result(dock, 'Must not have dangles')
    click_run(dock)
    assert visible('point') == 2
    assert visible('line') == 3


# Selecting an error


def highlights(canvas: t.Any) -> list[QgsHighlight]:
    return [
        item
        for item in canvas.scene().items()
        if isinstance(item, QgsHighlight)
    ]


def test_selecting_an_error_keeps_a_highlight(
    overlaps: Dock, qgis_iface: QgisInterface
):
    canvas = qgis_iface.mapCanvas()
    select_error(overlaps, 0)
    # Longer than a flash, which lasts half a second
    deadline = time.monotonic() + 0.6
    while time.monotonic() < deadline:
        QApplication.processEvents()
    assert len(highlights(canvas)) == 1


def test_the_next_selection_replaces_the_highlight(
    overlaps: Dock, qgis_iface: QgisInterface
):
    canvas = qgis_iface.mapCanvas()
    select_error(overlaps, 0)
    select_error(overlaps, 3)
    assert len(highlights(canvas)) == 1


def test_the_zoom_leaves_room_around_the_error(
    overlaps: Dock, qgis_iface: QgisInterface
):
    canvas = qgis_iface.mapCanvas()
    layer = error_layer('line')
    canvas.setDestinationCrs(layer.crs())
    select_error(overlaps, 0)
    extent = canvas.extent()
    # Every error is a segment 5 long
    assert extent.width() >= 1.5 * 5
    boxes = [f.geometry().boundingBox() for f in layer.getFeatures()]
    assert any(extent.contains(box) for box in boxes)


def test_the_zoom_of_a_point_still_shows_the_point(
    mixed: Dock, qgis_iface: QgisInterface
):
    canvas = qgis_iface.mapCanvas()
    layer = error_layer('point')
    canvas.setDestinationCrs(layer.crs())
    select_error(mixed, 0)
    extent = canvas.extent()
    assert extent.width() > 0
    points = [f.geometry().asPoint() for f in layer.getFeatures()]
    assert any(extent.contains(point) for point in points)


# The select button and the menu


def test_the_select_button_is_named(dock: Dock):
    assert dock.select_button.text() == 'Select features'


def test_the_select_button_needs_an_error(overlaps: Dock):
    assert not overlaps.select_button.isEnabled()
    select_error(overlaps, 0)
    assert overlaps.select_button.isEnabled()


def test_the_select_button_needs_an_error_not_a_rule(overlaps: Dock):
    select_result(overlaps, 'Must not overlap with')
    assert not overlaps.select_button.isEnabled()


def test_the_select_button_follows_the_selection_away(overlaps: Dock):
    select_error(overlaps, 0)
    overlaps.results_view.setCurrentIndex(QModelIndex())
    overlaps.results_view.clearSelection()
    assert not overlaps.select_button.isEnabled()


def checked_layers() -> tuple[QgsVectorLayer, QgsVectorLayer]:
    project = QgsProject.instance()
    first = project.mapLayersByName('first')[0]
    second = project.mapLayersByName('second')[0]
    assert isinstance(first, QgsVectorLayer)
    assert isinstance(second, QgsVectorLayer)
    return first, second


def test_selecting_features_selects_the_origin_and_the_other_feature(
    overlaps: Dock,
):
    first, second = checked_layers()
    select_error(overlaps, 4)
    fid = int(cell(overlaps, 4, FEATURE))
    other = int(cell(overlaps, 4, OTHER_FEATURE))
    overlaps.select_button.click()
    assert set(first.selectedFeatureIds()) == {fid}
    assert set(second.selectedFeatureIds()) == {other}


def test_selecting_features_replaces_the_selection(overlaps: Dock):
    first, second = checked_layers()
    first.selectByIds([1, 2, 3])
    second.selectByIds([4, 5])
    select_error(overlaps, 0)
    overlaps.select_button.click()
    assert len(first.selectedFeatureIds()) == 1
    assert len(second.selectedFeatureIds()) == 1


def test_selecting_the_features_of_a_one_layer_rule(dock: Dock, tmp_path: Path):
    roads = project_lines(tmp_path, (*CHAIN, 'LINESTRING(0 50, 10 50)'))
    roads.selectByIds([1, 2, 3])
    add(dock, DANGLES, roads)
    click_run(dock)
    select_error(dock, 0)
    fid = int(cell(dock, 0, FEATURE))
    dock.select_button.click()
    assert set(roads.selectedFeatureIds()) == {fid}


def test_the_context_menu_has_zoom_select_and_ignore(overlaps: Dock):
    view = overlaps.results_view
    assert view.contextMenuPolicy() == Qt.ContextMenuPolicy.ActionsContextMenu
    texts = [action.text() for action in view.actions()]
    assert texts[:2] == ['Zoom to error', 'Select features']
    assert 'Ignore error' in texts


def test_the_zoom_action_zooms_again(overlaps: Dock, qgis_iface: QgisInterface):
    canvas = qgis_iface.mapCanvas()
    canvas.setDestinationCrs(error_layer('line').crs())
    select_error(overlaps, 1)
    canvas.setExtent(QgsRectangle(-5000, -5000, 5000, 5000))
    overlaps.results_view.actions()[0].trigger()
    assert canvas.extent().width() < 1000


def test_the_select_action_selects(overlaps: Dock):
    first, _ = checked_layers()
    select_error(overlaps, 1)
    overlaps.results_view.actions()[1].trigger()
    assert len(first.selectedFeatureIds()) == 1


# The group of the error layers


def plugin_group_of(dock: Dock) -> t.Any:
    """The group of the layers that hold the errors of the last run."""
    project = QgsProject.instance()
    for node in project.layerTreeRoot().findLayers():
        layer = node.layer()
        if layer is not None and layer.fields().indexOf('violation_id') >= 0:
            return node.parent()
    return None


def test_the_errors_are_in_a_group(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_lines(tmp_path, CHAIN))
    click_run(dock)
    group = plugin_group_of(dock)
    assert group is not None
    assert group is not QgsProject.instance().layerTreeRoot()


def test_a_group_of_the_user_with_the_same_name_survives_a_rerun(
    dock: Dock, tmp_path: Path
):
    project = QgsProject.instance()
    mine = line_layer(*CHAIN, name='mine')
    project.addMapLayer(mine, False)
    group = project.layerTreeRoot().addGroup('Topology errors')
    group.addLayer(mine)
    add(dock, DANGLES, project_lines(tmp_path, CHAIN))
    click_run(dock)
    click_run(dock)
    assert project.mapLayer(mine.id()) is not None
    node = project.layerTreeRoot().findLayer(mine.id())
    assert isinstance(node, QgsLayerTreeLayer)
    assert node.parent().name() == 'Topology errors'
    assert shown(dock) == 2


def test_a_rerun_replaces_the_group_of_the_plugin(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_lines(tmp_path, CHAIN))
    click_run(dock)
    click_run(dock)
    holders = [
        layer
        for layer in QgsProject.instance().mapLayers().values()
        if layer.fields().indexOf('violation_id') >= 0
    ]
    assert len(holders) == 1


def test_a_rerun_after_the_user_deleted_the_group(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_lines(tmp_path, CHAIN))
    click_run(dock)
    group = plugin_group_of(dock)
    group.parent().removeChildNode(group)
    click_run(dock)
    assert shown(dock) == 2
    assert plugin_group_of(dock) is not None


# Errors without features belong to their entry as well


def test_gaps_are_grouped_under_their_layer_and_rule(dock: Dock):
    gaps_dock(dock)
    (node,) = result_nodes(dock, 0)
    assert node_text(node) == 'parcels: Must not have gaps (2)'


def test_no_node_of_the_results_starts_without_a_layer(dock: Dock):
    gaps_dock(dock)
    for node in result_nodes(dock, 0):
        assert not node_text(node).startswith(':')
        assert not node_text(node).startswith(' ')


def test_the_same_gaps_rule_on_two_layers_has_each_its_own_errors(dock: Dock):
    add(dock, GAPS, parcels('single', ring(0)))
    add(dock, GAPS, parcels('double', ring(0), ring(20)))
    click_run(dock)
    texts = [node_text(node) for node in result_nodes(dock, 0)]
    assert texts == [
        'single: Must not have gaps (2)',
        'double: Must not have gaps (4)',
    ]
    model = dock.results_view.model()
    counts = [
        sum(
            model.rowCount(kind)
            for kind in (
                model.index(row, 0, node) for row in range(model.rowCount(node))
            )
        )
        for node in result_nodes(dock, 0)
    ]
    assert counts == [2, 4]


def test_the_map_legend_names_the_entry_of_the_gaps(dock: Dock):
    add(dock, GAPS, parcels('single', ring(0)))
    add(dock, GAPS, parcels('double', ring(0), ring(20)))
    click_run(dock)
    # The gap and the outer boundary are lines
    renderer = error_layer('line').renderer()
    labels = {category.label() for category in renderer.categories()}
    assert labels == {
        'single: Must not have gaps',
        'double: Must not have gaps',
    }


# The order of the nodes is the order of the list

BOWTIE = 'POLYGON((20 0, 30 10, 30 0, 20 10, 20 0))'
SQUARE = 'POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))'
OVERLAPPING = 'POLYGON((2 2, 6 2, 6 6, 2 6, 2 2))'
POLYGON_OVERLAPS = 'polygon/must-not-overlap'


def test_the_nodes_follow_the_order_of_the_list(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, project_lines(tmp_path, CHAIN, 'tracks'))
    add(dock, DANGLES, project_lines(tmp_path, CHAIN, 'roads'))
    click_run(dock)
    texts = [node_text(node) for node in result_nodes(dock, 0)]
    assert texts == [
        'tracks: Must not have dangles (2)',
        'roads: Must not have dangles (2)',
    ]


def test_the_nodes_follow_the_list_when_the_second_rule_has_more(
    dock: Dock, tmp_path: Path
):
    wkts = (*CHAIN, 'LINESTRING(0 50, 10 50)')
    add(dock, DANGLES, project_lines(tmp_path, CHAIN, 'few'))
    add(dock, DANGLES, project_lines(tmp_path, wkts, 'many'))
    click_run(dock)
    assert [node_text(node)[:4] for node in result_nodes(dock, 0)] == [
        'few:',
        'many',
    ]


def test_invalid_geometry_comes_last(dock: Dock):
    broken = parcels('parcels', (SQUARE, OVERLAPPING, BOWTIE))
    add(dock, POLYGON_OVERLAPS, broken)
    add(dock, POLYGON_OVERLAPS, parcels('plots', (SQUARE, OVERLAPPING)))
    click_run(dock)
    texts = [node_text(node) for node in result_nodes(dock, 0)]
    assert len(texts) == 3
    assert 'Invalid geometry' in texts[-1]
    assert all('Invalid geometry' not in text for text in texts[:-1])
