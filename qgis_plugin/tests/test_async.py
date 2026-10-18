"""The dock asks the tool for its rules without freezing the interface."""

from __future__ import annotations

import collections.abc as c
import json
import time
from pathlib import Path

import pytest
from qgis.core import QgsProject
from qgis.gui import QgisInterface
from qgis.PyQt.QtWidgets import QApplication, QLabel

from topology_rules.core import binary
from topology_rules.panel.dock import Dock

from .helpers import RULES, line_layer

# The longest that the interface may stand still, and how slow the tool is
MAX_STALL = 0.2
SLOW = 1.0


def slow_tool(directory: Path, seconds: float) -> Path:
    """A tool that takes its time to list its rules."""
    path = directory / 'topology-checker'
    path.write_text(
        '#!/usr/bin/env python3\n'
        'import sys, time\n'
        "if sys.argv[1:] == ['--json', 'rules']:\n"
        f'    time.sleep({seconds})\n'
        f'    print({json.dumps(json.dumps(RULES))})\n'
        '    sys.exit(0)\n'
        'sys.exit(2)\n'
    )
    path.chmod(0o755)
    return path


@pytest.fixture
def slow_dock(
    qgis_iface: QgisInterface,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
) -> c.Iterator[Dock]:
    """A dock in front of a tool that needs a second, and a layer to check."""
    configure(slow_tool(tmp_path, SLOW))
    QgsProject.instance().addMapLayer(line_layer('LINESTRING(0 0, 1 1)'))
    dock = Dock(qgis_iface)
    dock.resize(300, 900)
    yield dock
    dock.cancel()
    dock.deleteLater()


def checking(dock: Dock) -> bool:
    return any(
        label.isVisible() and 'Checking' in label.text()
        for label in dock.findChildren(QLabel)
    )


def ready(dock: Dock) -> bool:
    return dock.rule_box.count() > 0


def wait_watching(dock: Dock, done: c.Callable[[], bool]) -> float:
    """Runs the event loop until done and returns the longest single step."""
    longest = 0.0
    deadline = time.monotonic() + 30
    while not done():
        assert time.monotonic() < deadline, 'timed out'
        before = time.monotonic()
        QApplication.processEvents()
        longest = max(longest, time.monotonic() - before)
        time.sleep(0.01)
    return longest


def test_showing_the_dock_does_not_wait_for_the_tool(slow_dock: Dock):
    started = time.monotonic()
    slow_dock.show()
    slow_dock.refresh()
    assert time.monotonic() - started < MAX_STALL


def test_the_event_loop_keeps_running_while_the_tool_answers(
    slow_dock: Dock,
):
    slow_dock.show()
    slow_dock.refresh()
    started = time.monotonic()
    longest = wait_watching(slow_dock, lambda: ready(slow_dock))
    assert time.monotonic() - started >= SLOW / 2
    assert longest < MAX_STALL


def test_the_dock_says_that_it_is_checking(slow_dock: Dock):
    slow_dock.show()
    slow_dock.refresh()
    QApplication.processEvents()
    assert checking(slow_dock)
    assert not ready(slow_dock)


def test_the_form_comes_when_the_tool_has_answered(slow_dock: Dock):
    slow_dock.show()
    slow_dock.refresh()
    wait_watching(slow_dock, lambda: ready(slow_dock))
    assert not checking(slow_dock)
    assert slow_dock.setup_page.isHidden()
    assert not slow_dock.tabs.isHidden()


def test_the_form_is_disabled_while_the_tool_is_checked(slow_dock: Dock):
    slow_dock.show()
    slow_dock.refresh()
    QApplication.processEvents()
    assert not slow_dock.add_button.isEnabled()


def test_a_tool_that_takes_too_long_is_given_up_without_a_freeze(
    qgis_iface: QgisInterface,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    monkeypatch: pytest.MonkeyPatch,
):
    monkeypatch.setattr(binary, 'PROBE_TIMEOUT', 0.3)
    configure(slow_tool(tmp_path, 10))
    dock = Dock(qgis_iface)
    dock.resize(300, 900)
    dock.show()
    dock.refresh()
    longest = wait_watching(dock, lambda: not dock.setup_page.isHidden())
    assert longest < MAX_STALL
    dock.deleteLater()


def test_the_wait_for_the_tool_is_long_by_default():
    assert binary.PROBE_TIMEOUT >= 30
