"""Cancelling a run ends the program of the tool, and every program that it
started. On Windows a tool that is a launcher starts another program, and
ending the launcher alone would leave that one running."""

from __future__ import annotations

import collections.abc as c
from pathlib import Path

import pytest
from qgis.core import QgsProject

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    gated_executable,
    process_alive,
    saved_layer,
    wait_for,
)

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


@pytest.fixture
def running(
    dock: Dock, configure: c.Callable[[str | Path], None], tmp_path: Path
) -> int:
    """A run that is on, with a tool that waits at a gate that never opens.
    Returns the process number of the tool."""
    wait_for(lambda: dock.probing is None)
    gate, started = tmp_path / 'gate', tmp_path / 'started'
    configure(gated_executable(tmp_path, gate, started))
    dock.refresh()
    wait_for(lambda: dock.probing is None and dock.available)
    (tmp_path / 'data').mkdir()
    layer = saved_layer(tmp_path / 'data', CHAIN, 'roads')
    QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, layer)
    dock.run_button.click()
    wait_for(lambda: started.exists() and started.read_text().isdigit())
    number = int(started.read_text())
    assert process_alive(number)
    return number


def test_cancelling_a_run_ends_the_program_of_the_tool(
    dock: Dock, running: int
):
    dock.run_button.click()
    wait_for(lambda: dock.task is None)
    wait_for(lambda: not process_alive(running), 30)


def test_cancelling_through_the_feedback_ends_the_program_of_the_tool(
    dock: Dock, running: int
):
    dock.cancel()
    wait_for(lambda: dock.task is None)
    wait_for(lambda: not process_alive(running), 30)
