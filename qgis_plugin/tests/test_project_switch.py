"""A run that is on when the user clears the project or opens another one: the
errors it finds are about layers that are gone, and are not for the new
project."""

from __future__ import annotations

import collections.abc as c
from pathlib import Path

import pytest
from qgis.core import QgsApplication, QgsProject, QgsVectorLayer

from topology_rules.panel.dock import Dock
from topology_rules.panel.groups import group_name

from .helpers import (
    add,
    error_layers,
    gated_executable,
    queue_column,
    saved_layer,
    shown,
    wait_for,
)

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


def in_project(directory: Path, name: str) -> QgsVectorLayer:
    directory.mkdir(parents=True, exist_ok=True)
    layer = saved_layer(directory, CHAIN, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


@pytest.fixture
def held(
    dock: Dock, configure: c.Callable[[str | Path], None], tmp_path: Path
) -> tuple[Path, Path]:
    """A dock whose tool begins a check, and waits until the gate file exists
    to do it. Returns the gate and the file that says the tool began."""
    wait_for(lambda: dock.probing is None)
    gate, started = tmp_path / 'gate', tmp_path / 'started'
    configure(gated_executable(tmp_path, gate, started))
    dock.refresh()
    wait_for(lambda: dock.probing is None and dock.available)
    return gate, started


def run_and_switch(
    dock: Dock,
    held: tuple[Path, Path],
    switch: c.Callable[[], object],
    tmp_path: Path,
) -> None:
    """Starts a run on a layer, changes the project while the tool is
    checking, and lets the tool end."""
    gate, started = held
    add(dock, DANGLES, in_project(tmp_path / 'old', 'roads'))
    dock.run_button.click()
    wait_for(started.exists)
    switch()
    gate.touch()
    wait_for(
        lambda: (
            dock.task is None and not QgsApplication.taskManager().activeTasks()
        )
    )
    for _ in range(20):
        QgsApplication.processEvents()


def another_project(dock: Dock, tmp_path: Path) -> Path:
    """A saved project with a rule of its own, left as the empty project."""
    add(dock, DANGLES, in_project(tmp_path / 'new', 'rails'))
    path = tmp_path / 'other.qgz'
    assert QgsProject.instance().write(str(path))
    QgsProject.instance().clear()
    return path


# The project is cleared


def test_the_errors_of_a_cleared_project_are_not_added_to_the_new_one(
    dock: Dock, held: tuple[Path, Path], tmp_path: Path
):
    run_and_switch(dock, held, QgsProject.instance().clear, tmp_path)
    assert error_layers() == []


def test_the_group_of_errors_is_not_added_to_the_new_project(
    dock: Dock, held: tuple[Path, Path], tmp_path: Path
):
    run_and_switch(dock, held, QgsProject.instance().clear, tmp_path)
    root = QgsProject.instance().layerTreeRoot()
    assert root.findGroup(group_name()) is None


def test_the_results_are_empty_after_the_project_was_cleared(
    dock: Dock, held: tuple[Path, Path], tmp_path: Path
):
    run_and_switch(dock, held, QgsProject.instance().clear, tmp_path)
    assert shown(dock) == 0


def test_the_dock_can_run_again_after_the_project_was_cleared(
    dock: Dock, held: tuple[Path, Path], tmp_path: Path
):
    run_and_switch(dock, held, QgsProject.instance().clear, tmp_path)
    assert dock.task is None
    assert dock.rules_page.isEnabled()
    assert dock.progress.isHidden()


# Another project is opened


def test_the_errors_of_the_old_project_are_not_added_to_the_opened_one(
    dock: Dock, held: tuple[Path, Path], tmp_path: Path
):
    path = another_project(dock, tmp_path)
    run_and_switch(
        dock, held, lambda: QgsProject.instance().read(str(path)), tmp_path
    )
    assert error_layers() == []


def test_the_counts_of_the_old_run_do_not_land_on_the_opened_rules(
    dock: Dock, held: tuple[Path, Path], tmp_path: Path
):
    path = another_project(dock, tmp_path)
    run_and_switch(
        dock, held, lambda: QgsProject.instance().read(str(path)), tmp_path
    )
    assert queue_column(dock, 1) == ['rails']
    assert queue_column(dock, 3) == ['']


def test_the_opened_project_has_no_results_of_the_old_run(
    dock: Dock, held: tuple[Path, Path], tmp_path: Path
):
    path = another_project(dock, tmp_path)
    run_and_switch(
        dock, held, lambda: QgsProject.instance().read(str(path)), tmp_path
    )
    assert shown(dock) == 0
