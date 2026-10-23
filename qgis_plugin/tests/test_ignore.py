"""Errors that the user knows and accepts: ignoring them, and getting them
back.

An error is told by its entry (rule, layer and other layer), its type, the
features that it names and a fingerprint of its geometry. So an ignored error
stays ignored when the same run is made again, and comes back when the
feature that it is about has moved."""

from __future__ import annotations

import typing as t
from pathlib import Path

from qgis.core import QgsGeometry, QgsProject, QgsVectorLayer
from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtWidgets import QAction, QCheckBox

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    both_engines,
    click_run,
    line_layer,
    message_text,
    node_text,
    queue_column,
    result_nodes,
    result_rows,
    saved_layer,
    saved_points,
    select_error,
    select_result,
    shown,
)

pytestmark = both_engines

DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
OVERLAPS = 'line/must-not-overlap-with'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


def project_lines(
    tmp_path: Path, name: str = 'roads', wkts: tuple[str, ...] = CHAIN
) -> QgsVectorLayer:
    tmp_path.mkdir(parents=True, exist_ok=True)
    layer = saved_layer(tmp_path, wkts, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def run_three(dock: Dock, tmp_path: Path) -> QgsVectorLayer:
    """Two dangles and a point that is not on the roads: three errors."""
    roads = project_lines(tmp_path)
    # Saved, as a memory layer is empty when a project is read again
    stops = saved_points(tmp_path, ['POINT(5 0)', 'POINT(5 5)'])
    QgsProject.instance().addMapLayer(stops)
    add(dock, DANGLES, roads)
    add(dock, COVERED, stops, roads)
    click_run(dock)
    return roads


def action(dock: Dock, text: str) -> QAction:
    return next(a for a in dock.results_view.actions() if a.text() == text)


def ignore(dock: Dock, row: int = 0) -> None:
    select_error(dock, row)
    action(dock, 'Ignore error').trigger()


def greyed(leaf: object) -> bool:
    """Whether an error of the results is drawn as one that is ignored."""
    return (
        not leaf.flags() & Qt.ItemFlag.ItemIsEnabled  # type: ignore[attr-defined]
        or leaf.data(Qt.ItemDataRole.ForegroundRole) is not None  # type: ignore[attr-defined]
    )


def ignored_row(dock: Dock) -> int:
    """The place of the error that is shown greyed out."""
    leaves = result_nodes(dock, 2)
    return next(i for i, leaf in enumerate(leaves) if greyed(leaf))


def visible(geometry_type: int) -> int:
    for layer in QgsProject.instance().mapLayers().values():
        if (
            isinstance(layer, QgsVectorLayer)
            and layer.fields().indexOf('violation_id') >= 0
            and int(layer.geometryType()) == geometry_type
        ):
            return int(layer.featureCount())
    raise AssertionError('no layer of errors')


# Ignoring one error


def test_the_menu_of_the_results_can_ignore_an_error(
    dock: Dock, tmp_path: Path
):
    run_three(dock, tmp_path)
    assert 'Ignore error' in [a.text() for a in dock.results_view.actions()]


def test_an_ignored_error_leaves_the_results(dock: Dock, tmp_path: Path):
    run_three(dock, tmp_path)
    ignore(dock)
    assert shown(dock) == 2


def test_an_ignored_error_leaves_the_map(dock: Dock, tmp_path: Path):
    run_three(dock, tmp_path)
    assert visible(0) == 3
    ignore(dock)
    assert visible(0) == 2


def test_the_other_errors_stay(dock: Dock, tmp_path: Path):
    run_three(dock, tmp_path)
    before = result_rows(dock)
    ignore(dock, 0)
    assert result_rows(dock) == before[1:]


def test_an_ignored_error_stays_ignored_when_the_run_is_made_again(
    dock: Dock, tmp_path: Path
):
    run_three(dock, tmp_path)
    ignore(dock)
    click_run(dock)
    assert shown(dock) == 2
    assert visible(0) == 2


def test_an_ignored_error_comes_back_when_its_feature_moves(
    dock: Dock, tmp_path: Path
):
    roads = run_three(dock, tmp_path)
    rows = result_rows(dock)
    # The dangle at the free end of the second road
    row = next(i for i, r in enumerate(rows) if r[2] == 'roads' and r[3] == '2')
    ignore(dock, row)
    assert shown(dock) == 2
    assert roads.startEditing()
    assert roads.changeGeometry(
        2, QgsGeometry.fromWkt('LINESTRING(10 0, 21 0)')
    )
    assert roads.commitChanges()
    click_run(dock)
    assert shown(dock) == 3


def test_an_error_of_another_entry_with_the_same_numbers_is_not_ignored(
    dock: Dock, tmp_path: Path
):
    add(dock, DANGLES, project_lines(tmp_path / 'a', 'roads'))
    add(dock, DANGLES, project_lines(tmp_path / 'b', 'tracks'))
    click_run(dock)
    assert shown(dock) == 4
    ignore(dock, 0)
    click_run(dock)
    assert shown(dock) == 3
    assert queue_column(dock, 3) == ['1', '2']


# Showing them again


def test_show_ignored_is_a_check_box_with_a_count(dock: Dock, tmp_path: Path):
    assert isinstance(dock.show_ignored, QCheckBox)
    run_three(dock, tmp_path)
    assert dock.show_ignored.text() == 'Show ignored (0)'
    ignore(dock)
    assert dock.show_ignored.text() == 'Show ignored (1)'


def test_ignored_errors_are_hidden_by_default(dock: Dock, tmp_path: Path):
    run_three(dock, tmp_path)
    ignore(dock)
    assert not dock.show_ignored.isChecked()


def test_showing_the_ignored_errors_lists_them_again(
    dock: Dock, tmp_path: Path
):
    run_three(dock, tmp_path)
    ignore(dock)
    dock.show_ignored.setChecked(True)
    assert shown(dock) == 3


def test_hiding_them_again_removes_them(dock: Dock, tmp_path: Path):
    run_three(dock, tmp_path)
    ignore(dock)
    dock.show_ignored.setChecked(True)
    dock.show_ignored.setChecked(False)
    assert shown(dock) == 2


def test_the_ignored_errors_are_greyed_out(dock: Dock, tmp_path: Path):
    run_three(dock, tmp_path)
    ignore(dock)
    dock.show_ignored.setChecked(True)
    assert sum(greyed(leaf) for leaf in result_nodes(dock, 2)) == 1


def test_the_menu_of_an_ignored_error_can_stop_ignoring(
    dock: Dock, tmp_path: Path
):
    run_three(dock, tmp_path)
    ignore(dock)
    dock.show_ignored.setChecked(True)
    select_error(dock, ignored_row(dock))
    assert action(dock, 'Stop ignoring').isEnabled()


def test_stopping_to_ignore_brings_the_error_back(dock: Dock, tmp_path: Path):
    run_three(dock, tmp_path)
    ignore(dock)
    dock.show_ignored.setChecked(True)
    select_error(dock, ignored_row(dock))
    action(dock, 'Stop ignoring').trigger()
    dock.show_ignored.setChecked(False)
    assert shown(dock) == 3
    assert dock.show_ignored.text() == 'Show ignored (0)'
    assert visible(0) == 3


# The counts


def test_the_count_of_a_rule_leaves_the_ignored_out(dock: Dock, tmp_path: Path):
    run_three(dock, tmp_path)
    assert queue_column(dock, 3) == ['2', '1']
    # The first rows are the dangles, as the first rule of the list is theirs
    ignore(dock, 0)
    assert queue_column(dock, 3) == ['1', '1']


def test_the_message_of_a_run_leaves_the_ignored_out_and_mentions_them(
    dock: Dock, tmp_path: Path, bar: t.Any
):
    run_three(dock, tmp_path)
    ignore(dock)
    bar.clear_messages()
    click_run(dock)
    assert '2 errors in 2 rules · 1 ignored' in message_text(bar)


def test_a_run_without_ignored_errors_does_not_mention_them(
    dock: Dock, tmp_path: Path, bar: t.Any
):
    run_three(dock, tmp_path)
    assert 'ignored' not in message_text(bar)


# A whole type at once


def many_overlaps(dock: Dock) -> None:
    first = line_layer(
        *[f'LINESTRING(0 {y}, 10 {y})' for y in range(12)], name='first'
    )
    second = line_layer(
        *[f'LINESTRING(5 {y}, 15 {y})' for y in range(12)], name='second'
    )
    for layer in (first, second):
        QgsProject.instance().addMapLayer(layer)
    add(dock, OVERLAPS, first, second)
    click_run(dock)


def test_ignoring_a_type_ignores_all_its_errors(dock: Dock):
    many_overlaps(dock)
    assert shown(dock) == 12
    select_result(dock, 'Overlaps')
    action(dock, 'Ignore error').trigger()
    assert shown(dock) == 0


def test_the_count_of_the_ignored_includes_the_whole_type(dock: Dock):
    many_overlaps(dock)
    select_result(dock, 'Overlaps')
    action(dock, 'Ignore error').trigger()
    assert dock.show_ignored.text() == 'Show ignored (12)'


def test_a_type_that_was_ignored_stays_ignored_after_a_rerun(dock: Dock):
    many_overlaps(dock)
    select_result(dock, 'Overlaps')
    action(dock, 'Ignore error').trigger()
    click_run(dock)
    assert shown(dock) == 0
    assert [node_text(n) for n in result_nodes(dock, 0)] == []


# The project


def test_the_ignored_errors_are_saved_with_the_project(
    dock: Dock, tmp_path: Path
):
    run_three(dock, tmp_path)
    ignore(dock)
    path = tmp_path / 'project.qgz'
    assert QgsProject.instance().write(str(path))
    QgsProject.instance().clear()
    assert QgsProject.instance().read(str(path))
    click_run(dock)
    assert shown(dock) == 2
    assert dock.show_ignored.text() == 'Show ignored (1)'


def test_a_new_project_ignores_nothing(dock: Dock, tmp_path: Path):
    run_three(dock, tmp_path)
    ignore(dock)
    QgsProject.instance().clear()
    assert dock.show_ignored.text() == 'Show ignored (0)'
