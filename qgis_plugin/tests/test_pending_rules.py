"""A project that is opened while the tool is not there: the panel cannot name
its rules yet, and keeps them for when it can. Changing an option and saving
the project must not lose them."""

from __future__ import annotations

import collections.abc as c
from pathlib import Path

import pytest
from qgis.core import QgsProject

from topology_rules.panel.dock import Dock

from .helpers import add, queue_column, saved_layer, wait_for

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


def change_tolerance(dock: Dock) -> None:
    dock.tolerance.setValue(0.5)


def change_selected(dock: Dock) -> None:
    dock.selected_only.setChecked(True)


def change_extent(dock: Dock) -> None:
    dock.extent_only.setChecked(True)


def same_tolerance(dock: Dock) -> bool:
    return dock.tolerance.value() == 0.5


CHANGES = {
    'tolerance': (change_tolerance, same_tolerance),
    'selected': (change_selected, lambda dock: dock.selected_only.isChecked()),
    'extent': (change_extent, lambda dock: dock.extent_only.isChecked()),
}


@pytest.mark.parametrize('change', CHANGES)
def test_the_rules_of_a_project_survive_a_change_made_while_the_tool_is_missing(
    dock: Dock,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    executable: str,
    change: str,
):
    apply, kept = CHANGES[change]
    (tmp_path / 'data').mkdir()
    layer = saved_layer(tmp_path / 'data', CHAIN, 'roads')
    QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, layer)
    first, second = tmp_path / 'first.qgz', tmp_path / 'second.qgz'
    assert QgsProject.instance().write(str(first))
    QgsProject.instance().clear()
    # The tool is gone when the project is opened
    configure(tmp_path / 'gone')
    dock.refresh()
    assert not dock.available
    assert QgsProject.instance().read(str(first))
    apply(dock)
    assert QgsProject.instance().write(str(second))
    QgsProject.instance().clear()
    configure(executable)
    dock.refresh()
    wait_for(lambda: dock.available)
    assert QgsProject.instance().read(str(second))
    assert queue_column(dock, 1) == ['roads']
    assert kept(dock)
