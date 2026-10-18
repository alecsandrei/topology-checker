"""Exporting the rules with the option of the selected features: the file that
is written checks what the dock checks, for layers that are files too."""

from __future__ import annotations

import tomllib
from pathlib import Path

import pytest
from qgis.core import QgsProject, QgsVectorLayer

from topology_rules.panel.dock import Dock

from .helpers import add, cli_counts, click_run, queue_column, saved_layer

DANGLES = 'line/must-not-have-dangles'
# Two lines that meet, and one far from them
LINES = (
    'LINESTRING(0 0, 10 0)',
    'LINESTRING(10 0, 20 0)',
    'LINESTRING(0 50, 10 50)',
)


@pytest.fixture
def data(tmp_path: Path) -> Path:
    folder = tmp_path / 'data'
    folder.mkdir()
    return folder


def in_project(data: Path, name: str, selected: list[int]) -> QgsVectorLayer:
    layer = saved_layer(data, LINES, name)
    QgsProject.instance().addMapLayer(layer)
    layer.selectByIds(selected)
    return layer


def dock_counts(dock: Dock) -> list[int]:
    click_run(dock)
    return [int(count) for count in queue_column(dock, 3)]


def test_the_file_checks_the_selected_features_of_a_layer_that_is_a_file(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    add(dock, DANGLES, in_project(data, 'roads', [1, 2]))
    dock.selected_only.setChecked(True)
    toml = tmp_path / 'topology.toml'
    dock.export_rules(toml)
    assert cli_counts(executable, toml) == [2]
    assert dock_counts(dock) == [2]


def test_a_layer_without_a_selection_adds_no_errors_to_the_file(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    add(dock, DANGLES, in_project(data, 'chosen', [1, 2]))
    add(dock, DANGLES, in_project(data, 'other', []))
    dock.selected_only.setChecked(True)
    toml = tmp_path / 'topology.toml'
    dock.export_rules(toml)
    assert cli_counts(executable, toml) == [2, 0]
    assert dock_counts(dock) == [2, 0]


def test_the_file_of_a_layer_is_not_changed_by_the_export(
    dock: Dock, data: Path, tmp_path: Path
):
    layer = in_project(data, 'roads', [1, 2])
    add(dock, DANGLES, layer)
    dock.selected_only.setChecked(True)
    before = Path(layer.source().partition('|')[0]).read_bytes()
    dock.export_rules(tmp_path / 'topology.toml')
    assert Path(layer.source().partition('|')[0]).read_bytes() == before


def test_without_the_option_the_file_checks_the_whole_layer(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    add(dock, DANGLES, in_project(data, 'roads', [1, 2]))
    toml = tmp_path / 'topology.toml'
    dock.export_rules(toml)
    assert cli_counts(executable, toml) == [4]
    assert dock_counts(dock) == [4]


def test_without_the_option_the_layer_stays_in_its_own_file(
    dock: Dock, data: Path, tmp_path: Path
):
    add(dock, DANGLES, in_project(data, 'roads', [1, 2]))
    toml = tmp_path / 'topology.toml'
    dock.export_rules(toml)
    config = tomllib.loads(toml.read_text(encoding='utf-8'))
    (path,) = (source['path'] for source in config['layers'].values())
    assert (toml.parent / path).resolve() == (data / 'roads.gpkg').resolve()
    assert not (tmp_path / 'topology_data.gpkg').exists()
