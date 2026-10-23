"""A layer that is only the second layer of a rule is renamed: the rule, the
project and the exported file follow, as they do for the first layer."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
import tomllib
from qgis.core import QgsProject, QgsVectorLayer

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    both_engines,
    queue_column,
    saved_layer,
    saved_points,
)

pytestmark = both_engines

COVERED = 'point/must-be-covered-by-lines'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
NEW = 'streets'


@pytest.fixture
def renamed(dock: Dock, tmp_path: Path) -> Dock:
    """Stops that must lie on the roads, and the roads renamed: they are only
    the other layer of the rule."""
    (tmp_path / 'data').mkdir()
    roads: QgsVectorLayer = saved_layer(tmp_path / 'data', CHAIN, 'roads')
    stops = saved_points(tmp_path / 'data', ['POINT(5 0)'], 'stops')
    QgsProject.instance().addMapLayer(roads)
    QgsProject.instance().addMapLayer(stops)
    add(dock, COVERED, stops, roads)
    roads.setName(NEW)
    return dock


def test_the_entry_has_the_new_name_of_the_other_layer(renamed: Dock):
    (entry,) = renamed.entries
    assert entry.other_name == NEW


def test_the_title_of_the_rule_has_the_new_name_of_the_other_layer(
    renamed: Dock,
):
    (entry,) = renamed.entries
    assert NEW in entry.title
    assert queue_column(renamed, 2) == [NEW]


def test_the_project_keeps_the_new_name_of_the_other_layer(renamed: Dock):
    text, found = QgsProject.instance().readEntry('TopologyRules', 'rules', '')
    assert found
    assert [item['other_name'] for item in json.loads(text)] == [NEW]


def test_the_exported_file_names_the_other_layer_by_its_new_name(
    renamed: Dock, tmp_path: Path
):
    toml = tmp_path / 'topology.toml'
    renamed.export_rules(toml)
    config = tomllib.loads(toml.read_text(encoding='utf-8'))
    assert NEW in config['layers']
    assert 'roads' not in config['layers']
