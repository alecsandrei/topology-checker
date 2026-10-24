"""The tabs and the run button of the dock, before, during and after a run."""

from __future__ import annotations

import collections.abc as c
import typing as t
from pathlib import Path

import pytest
from qgis.core import Qgis, QgsApplication, QgsProject
from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtTest import QTest
from qgis.PyQt.QtWidgets import QProgressBar, QTabWidget, QTreeWidgetItem

from topology_rules.core import binary, engine, folders
from topology_rules.core.engine import Package
from topology_rules.panel.dock import Dock
from topology_rules.utils import not_none

from .helpers import (
    add,
    both_engines,
    click_run,
    executable_only,
    saved_layer,
    script_executable,
    wait_for,
)

pytestmark = both_engines

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)


def lines(tmp_path: Path, wkts: tuple[str, ...], name: str = 'roads'):
    layer = saved_layer(tmp_path, wkts, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


@pytest.fixture
def running(
    dock: Dock,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
) -> Dock:
    """A dock in the middle of a run that does not end by itself."""
    configure(script_executable(tmp_path, 'import time\ntime.sleep(60)\n'))
    dock.refresh()
    add(dock, DANGLES, lines(tmp_path, CHAIN))
    dock.run_button.click()
    assert dock.task is not None
    return dock


@pytest.fixture
def logged() -> c.Iterator[list[str]]:
    """What reaches the Topology Rules tab of the Log Messages panel."""
    found: list[str] = []

    def received(message: str, tag: str, *rest: object) -> None:
        if tag == folders.TAG:
            found.append(message)

    log = not_none(QgsApplication.messageLog())
    # The signal that the panel listens to, which QGIS 4 renamed
    signal = getattr(log, 'messageReceivedWithFormat', log.messageReceived)
    signal.connect(received)
    yield found
    signal.disconnect(received)


def test_a_run_logs_its_engine(dock: Dock, tmp_path: Path, logged: list[str]):
    add(dock, DANGLES, lines(tmp_path, CHAIN))
    click_run(dock)
    used = engine.tool()
    line = engine.describe(used)
    assert logged.count(line) == 1
    if isinstance(used, Package):
        assert f' {used.version} ({used.location})' in line
    else:
        assert line.endswith(f'{used} {binary.version(used)}')


def test_the_dock_opens_on_rules_with_a_plain_errors_tab(dock: Dock):
    assert isinstance(dock.tabs, QTabWidget)
    names = [dock.tabs.tabText(index) for index in range(dock.tabs.count())]
    assert names == ['Rules', 'Errors']
    assert dock.tabs.currentIndex() == 0


def test_a_run_with_errors_counts_them_in_the_tab(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN))
    click_run(dock)
    assert dock.tabs.tabText(1) == 'Errors (2)'


def test_a_run_with_errors_shows_the_errors_tab(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN))
    click_run(dock)
    assert dock.tabs.currentIndex() == 1


def test_a_run_without_errors_stays_on_the_rules_tab(
    dock: Dock, tmp_path: Path
):
    add(dock, DANGLES, lines(tmp_path, RING))
    click_run(dock)
    assert dock.tabs.currentIndex() == 0
    assert dock.tabs.tabText(1) == 'Errors'


def test_a_run_without_errors_says_so(dock: Dock, tmp_path: Path, bar: t.Any):
    add(dock, DANGLES, lines(tmp_path, RING))
    click_run(dock)
    assert bar.get_messages(Qgis.MessageLevel.Success)


def test_a_clean_rerun_clears_the_count(dock: Dock, tmp_path: Path):
    roads = lines(tmp_path, CHAIN)
    add(dock, DANGLES, roads)
    click_run(dock)
    dock.clear_rules()
    add(dock, DANGLES, lines(tmp_path, RING, 'rails'))
    click_run(dock)
    assert dock.tabs.tabText(1) == 'Errors'


def test_the_run_button_names_the_rules_when_idle(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, lines(tmp_path, CHAIN))
    add(dock, DANGLES, lines(tmp_path, RING, 'rails'))
    assert dock.run_button.text() == 'Run 2 rules'


def test_there_is_no_separate_cancel_button(dock: Dock):
    assert not hasattr(dock, 'cancel_button')


@executable_only
def test_the_run_button_cancels_during_a_run(running: Dock):
    assert running.run_button.text() == 'Cancel'
    assert running.run_button.isEnabled()


@executable_only
def test_clicking_the_run_button_during_a_run_cancels_it(running: Dock):
    running.run_button.click()
    wait_for(lambda: running.task is None, timeout=20)
    assert running.run_button.text() == 'Run 1 rule'
    assert running.tabs.tabText(1) == 'Errors'


@executable_only
def test_a_canceled_run_leaves_the_rules_editable(running: Dock):
    running.run_button.click()
    wait_for(lambda: running.task is None, timeout=20)
    assert running.add_button.isEnabled()
    assert running.rule_box.isEnabled()


def test_the_progress_bar_is_hidden_when_idle(dock: Dock):
    assert isinstance(dock.progress, QProgressBar)
    assert dock.progress.isHidden()


@executable_only
def test_the_progress_bar_is_indeterminate_during_a_run(running: Dock):
    assert not running.progress.isHidden()
    assert running.progress.minimum() == running.progress.maximum() == 0


@executable_only
def test_the_progress_bar_goes_with_the_run(running: Dock):
    running.run_button.click()
    wait_for(lambda: running.task is None, timeout=20)
    assert running.progress.isHidden()


@pytest.mark.parametrize(
    'name',
    [
        'rule_box',
        'input_box',
        'other_box',
        'add_button',
        'tolerance',
        'remove_button',
        'clear_button',
        'import_button',
        'export_button',
    ],
)
@executable_only
def test_the_form_and_the_buttons_of_the_list_are_disabled_during_a_run(
    running: Dock, name: str
):
    assert not getattr(running, name).isEnabled()


def rule_item(dock: Dock) -> QTreeWidgetItem:
    return not_none(not_none(dock.rules_tree.topLevelItem(0)).child(0))


@executable_only
def test_the_list_of_rules_stays_readable_during_a_run(running: Dock):
    """It is not greyed out, as it shows the counts of the rules that are
    done, and its rules cannot be ticked or taken off."""
    tree = running.rules_tree
    assert tree.isEnabled()
    item = rule_item(running)
    assert item.flags() & Qt.ItemFlag.ItemIsEnabled
    assert not item.flags() & Qt.ItemFlag.ItemIsUserCheckable
    tree.setCurrentItem(item)
    QTest.keyClick(tree, Qt.Key.Key_Delete)
    assert len(running.entries) == 1
    assert rule_item(running).checkState(0) == Qt.CheckState.Checked


@executable_only
def test_the_rules_can_be_ticked_and_taken_off_after_a_run(running: Dock):
    running.run_button.click()
    wait_for(lambda: running.task is None, timeout=20)
    item = rule_item(running)
    assert item.flags() & Qt.ItemFlag.ItemIsUserCheckable
    item.setCheckState(0, Qt.CheckState.Unchecked)
    assert not running.entries[0].checked
    running.rules_tree.setCurrentItem(item)
    QTest.keyClick(running.rules_tree, Qt.Key.Key_Delete)
    assert running.entries == []
