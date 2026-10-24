"""The options that limit a run, the selected features and the visible extent,
limit the first layer of each rule: the layer that is checked. The other layer
of a rule is what the first is checked against, and is always used whole."""

from __future__ import annotations

from pathlib import Path

from qgis.core import QgsProject, QgsRectangle, QgsVectorLayer
from qgis.gui import QgisInterface

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    both_engines,
    cli_counts,
    click_run,
    line_layer,
    message_text,
    point_layer,
    polygon_layer,
    queue_column,
    shown,
)

pytestmark = both_engines

COVERED = 'point/must-be-covered-by-polygons'
OVERLAPS_WITH = 'line/must-not-overlap-with'
# Ten squares, of which the first is the only one that is selected
SQUARES = tuple(
    f'POLYGON(({x} 0, {x + 4} 0, {x + 4} 4, {x} 4, {x} 0))'
    for x in range(0, 100, 10)
)
# A point in each of the squares 1 to 5, and one that is in none of them
INSIDE = tuple(f'POINT({x + 2} 2)' for x in range(10, 60, 10))
OUTSIDE = 'POINT(500 500)'


def in_project(layer: QgsVectorLayer) -> QgsVectorLayer:
    QgsProject.instance().addMapLayer(layer)
    return layer


def points_and_squares(*points: str) -> tuple[QgsVectorLayer, QgsVectorLayer]:
    """The points, of which the first five are selected, and the squares, of
    which the first is selected."""
    chosen = in_project(point_layer(*points, name='points'))
    chosen.selectByIds([1, 2, 3, 4, 5])
    squares = in_project(polygon_layer(*SQUARES, name='squares'))
    squares.selectByIds([1])
    return chosen, squares


def run_selected(dock: Dock, *points: str) -> None:
    chosen, squares = points_and_squares(*points)
    add(dock, COVERED, chosen, squares)
    dock.selected_only.setChecked(True)
    click_run(dock)


# The selected features


def test_a_selected_point_in_a_square_that_is_not_selected_is_not_reported(
    dock: Dock,
):
    run_selected(dock, *INSIDE, OUTSIDE)
    assert shown(dock) == 0


def test_a_selected_point_outside_every_square_is_reported(dock: Dock):
    run_selected(dock, *INSIDE[:4], OUTSIDE)
    assert shown(dock) == 1


def test_a_point_that_is_not_selected_is_not_checked(dock: Dock):
    run_selected(dock, *INSIDE, OUTSIDE)
    assert queue_column(dock, 3) == ['0']


def test_the_message_does_not_ask_for_a_selection_in_the_other_layer(
    dock: Dock, bar: object
):
    squares = in_project(polygon_layer(*SQUARES, name='squares'))
    chosen = in_project(point_layer(*INSIDE, name='points'))
    chosen.selectByIds([1])
    add(dock, COVERED, chosen, squares)
    dock.selected_only.setChecked(True)
    click_run(dock)
    assert 'squares' not in message_text(bar)


def test_a_line_that_is_not_selected_is_the_other_layer_of_a_selected_one(
    dock: Dock,
):
    # The lines overlap from 50 to 100
    first = in_project(line_layer('LINESTRING(0 0, 100 0)', name='first'))
    other = in_project(line_layer('LINESTRING(50 0, 150 0)', name='other'))
    first.selectByIds([1])
    add(dock, OVERLAPS_WITH, first, other)
    dock.selected_only.setChecked(True)
    click_run(dock)
    assert shown(dock) >= 1


# The visible extent


def test_a_line_outside_the_extent_is_the_other_layer_of_one_inside_it(
    dock: Dock, qgis_iface: QgisInterface
):
    first = in_project(line_layer('LINESTRING(0 0, 100 0)', name='first'))
    other = in_project(line_layer('LINESTRING(50 0, 150 0)', name='other'))
    canvas = qgis_iface.mapCanvas()
    canvas.setDestinationCrs(first.crs())
    # Around the start of the first line only, away from the other line
    canvas.setExtent(QgsRectangle(-1, -5, 10, 5))
    add(dock, OVERLAPS_WITH, first, other)
    dock.extent_only.setChecked(True)
    click_run(dock)
    assert shown(dock) >= 1


# The rules that are exported


def exported_and_run(dock: Dock, tmp_path: Path, executable: str) -> list[int]:
    toml = tmp_path / 'topology.toml'
    dock.export_rules(toml)
    exported = cli_counts(executable, toml)
    click_run(dock)
    assert exported == [int(count) for count in queue_column(dock, 3)]
    return exported


def test_the_exported_file_uses_the_other_layer_whole(
    dock: Dock, tmp_path: Path, executable: str
):
    chosen, squares = points_and_squares(*INSIDE, OUTSIDE)
    add(dock, COVERED, chosen, squares)
    dock.selected_only.setChecked(True)
    assert exported_and_run(dock, tmp_path, executable) == [0]


def test_the_exported_file_reports_a_selected_point_outside_every_square(
    dock: Dock, tmp_path: Path, executable: str
):
    chosen, squares = points_and_squares(*INSIDE[:4], OUTSIDE)
    add(dock, COVERED, chosen, squares)
    dock.selected_only.setChecked(True)
    assert exported_and_run(dock, tmp_path, executable) == [1]
