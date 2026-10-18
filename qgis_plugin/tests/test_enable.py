"""Rules of the list can be switched off without leaving the list."""

from __future__ import annotations

from pathlib import Path

from qgis.core import QgsProject, QgsVectorLayer
from qgis.PyQt.QtCore import Qt

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    checked,
    click_run,
    queue_column,
    rule_items,
    saved_layer,
    select_rules,
    set_checked,
    shown,
)

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


def lines(tmp_path: Path, name: str) -> QgsVectorLayer:
    layer = saved_layer(tmp_path, CHAIN, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def two_rules(dock: Dock, tmp_path: Path) -> None:
    add(dock, DANGLES, lines(tmp_path, 'roads'))
    add(dock, DANGLES, lines(tmp_path, 'rails'))


def test_every_row_has_a_checkbox_that_is_checked(dock: Dock, tmp_path: Path):
    two_rules(dock, tmp_path)
    for item in rule_items(dock):
        assert item.flags() & Qt.ItemFlag.ItemIsUserCheckable
    assert checked(dock) == [True, True]


def test_the_run_button_counts_the_checked_rows(dock: Dock, tmp_path: Path):
    two_rules(dock, tmp_path)
    assert dock.run_button.text() == 'Run 2 rules'
    set_checked(dock, 1, False)
    assert dock.run_button.text() == 'Run 1 rule'


def test_run_is_disabled_when_nothing_is_checked(dock: Dock, tmp_path: Path):
    two_rules(dock, tmp_path)
    set_checked(dock, 0, False)
    set_checked(dock, 1, False)
    assert not dock.run_button.isEnabled()
    assert dock.run_button.text() == 'Run'


def test_checking_a_row_again_enables_run(dock: Dock, tmp_path: Path):
    two_rules(dock, tmp_path)
    set_checked(dock, 0, False)
    set_checked(dock, 1, False)
    set_checked(dock, 1, True)
    assert dock.run_button.isEnabled()
    assert dock.run_button.text() == 'Run 1 rule'


def test_run_uses_only_the_checked_rows(dock: Dock, tmp_path: Path):
    two_rules(dock, tmp_path)
    set_checked(dock, 1, False)
    click_run(dock)
    assert shown(dock) == 2
    assert queue_column(dock, 3) == ['2', '']


def test_a_row_that_was_not_run_has_no_count_in_the_errors_column(
    dock: Dock, tmp_path: Path
):
    two_rules(dock, tmp_path)
    click_run(dock)
    set_checked(dock, 0, False)
    click_run(dock)
    assert queue_column(dock, 3) == ['', '2']


def test_the_checkboxes_survive_a_run(dock: Dock, tmp_path: Path):
    two_rules(dock, tmp_path)
    set_checked(dock, 0, False)
    click_run(dock)
    assert checked(dock) == [False, True]


def test_removing_a_row_keeps_the_other_checkboxes(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, 'roads'))
    add(dock, DANGLES, lines(tmp_path, 'rails'))
    add(dock, DANGLES, lines(tmp_path, 'paths'))
    set_checked(dock, 2, False)
    select_rules(dock, 0)
    dock.remove_rules()
    assert checked(dock) == [True, False]


def test_a_rule_added_after_a_clear_is_checked(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, 'roads'))
    set_checked(dock, 0, False)
    dock.clear_rules()
    add(dock, DANGLES, lines(tmp_path, 'rails'))
    assert checked(dock) == [True]
