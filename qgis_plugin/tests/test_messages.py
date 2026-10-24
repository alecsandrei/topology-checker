"""What the dock tells the user when the tool fails."""

from __future__ import annotations

import collections.abc as c
import typing as t
from pathlib import Path

import pytest
from qgis.core import Qgis, QgsProject
from qgis.PyQt.QtWidgets import (
    QDialog,
    QLabel,
    QMessageBox,
    QPlainTextEdit,
    QPushButton,
    QTextEdit,
)

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    both_engines,
    click_run,
    duration_of,
    message_text,
    point_layer,
    polygon_layer,
    saved_layer,
    script_executable,
    wait_for,
)

DANGLES = 'line/must-not-have-dangles'
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
FAILURE = 'database is on fire'


@pytest.fixture
def failing(
    dock: Dock,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
) -> Path:
    """A dock whose tool answers `--json rules` but fails every check."""
    path = script_executable(
        tmp_path,
        f'print(\'{{"error": "{FAILURE}"}}\', file=sys.stderr)\nsys.exit(2)\n',
    )
    configure(path)
    dock.refresh()
    layer = saved_layer(tmp_path, CHAIN)
    QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, layer)
    click_run(dock)
    return path


def details_buttons(bar: t.Any) -> list[QPushButton]:
    return [
        button
        for button in bar.findChildren(QPushButton)
        if button.text() == 'Details'
    ]


def test_a_failure_shows_the_message_of_the_tool(failing: Path, bar: t.Any):
    assert FAILURE in message_text(bar)


def test_a_failure_does_not_show_the_path_of_the_tool(
    failing: Path, bar: t.Any
):
    assert str(failing) not in message_text(bar)


def test_a_failure_does_not_show_the_command_line(failing: Path, bar: t.Any):
    text = message_text(bar)
    assert '--json' not in text
    assert 'validate' not in text
    assert '--overwrite' not in text


def test_a_failure_leaves_the_dock_usable(failing: Path, dock: Dock):
    assert dock.task is None
    assert dock.run_button.isEnabled()
    assert dock.add_button.isEnabled()


def test_a_failure_has_a_details_button(failing: Path, bar: t.Any):
    assert len(details_buttons(bar)) == 1


@pytest.fixture
def dialogs(monkeypatch: pytest.MonkeyPatch) -> list[QDialog]:
    """The dialogs that are shown, which are not: a modal one would block."""
    shown: list[QDialog] = []

    def record(self: QDialog, *args: object) -> int:
        shown.append(self)
        return 0

    for cls in (QDialog, QMessageBox):
        for method in ('exec', 'show', 'open'):
            monkeypatch.setattr(cls, method, record)
    return shown


def dialog_text(dialog: QDialog) -> str:
    texts = [dialog.windowTitle()]
    for cls in (QTextEdit, QPlainTextEdit):
        texts += [edit.toPlainText() for edit in dialog.findChildren(cls)]
    texts += [label.text() for label in dialog.findChildren(QLabel)]
    if isinstance(dialog, QMessageBox):
        texts += [dialog.text(), dialog.detailedText()]
    return '\n'.join(texts)


def test_details_show_the_full_log(
    failing: Path, bar: t.Any, dialogs: list[QDialog]
):
    details_buttons(bar)[0].click()
    assert dialogs
    text = dialog_text(dialogs[-1])
    assert FAILURE in text
    assert str(failing) in text
    assert '--json' in text


def test_a_canceled_run_shows_no_error(
    dock: Dock,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    bar: t.Any,
):
    configure(script_executable(tmp_path, 'import time\ntime.sleep(60)\n'))
    dock.refresh()
    layer = saved_layer(tmp_path, CHAIN)
    QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, layer)
    dock.run_button.click()
    dock.run_button.click()
    wait_for(lambda: dock.task is None, timeout=20)
    assert not bar.get_messages(Qgis.MessageLevel.Critical)


# One message for each run

OVERLAPS = 'polygon/must-not-overlap'
SQUARE = 'POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))'
OVERLAPPING = 'POLYGON((2 2, 6 2, 6 6, 2 6, 2 2))'
BOWTIE = 'POLYGON((20 0, 30 10, 30 0, 20 10, 20 0))'
SECOND_BOWTIE = 'POLYGON((40 0, 50 10, 50 0, 40 10, 40 0))'


def messages(bar: t.Any) -> list[str]:
    return [
        text
        for level in (
            Qgis.MessageLevel.Info,
            Qgis.MessageLevel.Success,
            Qgis.MessageLevel.Warning,
            Qgis.MessageLevel.Critical,
        )
        for text in bar.get_messages(level)
    ]


def run_with_errors(dock: Dock, tmp_path: Path) -> None:
    roads = saved_layer(tmp_path, CHAIN, 'roads')
    stops = point_layer('POINT(5 0)', 'POINT(5 5)', name='stops')
    for layer in (roads, stops):
        QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, roads)
    add(dock, 'point/must-be-covered-by-lines', stops, roads)
    click_run(dock)


def run_with_invalid(dock: Dock) -> None:
    layer = polygon_layer(SQUARE, OVERLAPPING, BOWTIE, SECOND_BOWTIE)
    QgsProject.instance().addMapLayer(layer)
    add(dock, OVERLAPS, layer)
    click_run(dock)


@both_engines
def test_a_run_pushes_one_message(dock: Dock, tmp_path: Path, bar: t.Any):
    run_with_errors(dock, tmp_path)
    assert len(messages(bar)) == 1


@both_engines
def test_the_message_counts_the_errors_and_the_rules(
    dock: Dock, tmp_path: Path, bar: t.Any
):
    run_with_errors(dock, tmp_path)
    assert '3 errors in 2 rules' in messages(bar)[0]


@both_engines
def test_a_clean_run_pushes_one_success_message(
    dock: Dock, tmp_path: Path, bar: t.Any
):
    ring = saved_layer(tmp_path, RING, 'ring')
    QgsProject.instance().addMapLayer(ring)
    add(dock, DANGLES, ring)
    click_run(dock)
    assert len(bar.get_messages(Qgis.MessageLevel.Success)) == 1
    assert len(messages(bar)) == 1


@both_engines
def test_a_message_without_a_failure_stays_five_seconds(
    dock: Dock, tmp_path: Path, bar: t.Any
):
    run_with_errors(dock, tmp_path)
    assert [duration_of(call) for call in bar.calls] == [5]


@both_engines
def test_a_clean_message_stays_five_seconds(
    dock: Dock, tmp_path: Path, bar: t.Any
):
    ring = saved_layer(tmp_path, RING, 'ring')
    QgsProject.instance().addMapLayer(ring)
    add(dock, DANGLES, ring)
    click_run(dock)
    assert [duration_of(call) for call in bar.calls] == [5]


def test_a_failure_stays_until_it_is_closed(failing: Path, bar: t.Any):
    assert all(duration_of(call) <= 0 for call in bar.calls)


def test_a_new_failure_replaces_the_previous_one(
    failing: Path, dock: Dock, bar: t.Any
):
    click_run(dock)
    assert len(details_buttons(bar)) == 1


@both_engines
def test_the_message_adds_the_invalid_geometries_that_were_skipped(
    dock: Dock, bar: t.Any
):
    run_with_invalid(dock)
    assert len(messages(bar)) == 1
    assert '· 2 invalid geometries skipped' in messages(bar)[0]


@both_engines
def test_the_message_has_a_show_button_for_the_invalid_geometries(
    dock: Dock, bar: t.Any
):
    run_with_invalid(dock)
    buttons = [b for b in bar.findChildren(QPushButton) if b.text() == 'Show']
    assert len(buttons) == 1
    buttons[0].click()
    assert dock.tabs.currentIndex() == 1
    assert 'Invalid geometry' in dock.results_view.currentIndex().data()


@both_engines
def test_a_run_without_invalid_geometries_has_no_show_button(
    dock: Dock, tmp_path: Path, bar: t.Any
):
    run_with_errors(dock, tmp_path)
    assert not [b for b in bar.findChildren(QPushButton) if b.text() == 'Show']
    assert 'invalid' not in messages(bar)[0]
