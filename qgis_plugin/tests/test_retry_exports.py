"""A run is repeated when the tool rejects a feature. Only the layer with the
feature is written again: the others stay as they are, or keep their first
export."""

from __future__ import annotations

import collections.abc as c
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsProject, QgsVectorLayer

from topology_rules.core import inputs
from topology_rules.panel.dock import Dock

from .helpers import (
    RULES,
    add,
    click_run,
    layer_nodes,
    polygon_layer,
    save,
    script_executable,
    wait_for,
)
from .test_invalid import ELSEWHERE, SQUARE

OVERLAPS_WITH = 'polygon/must-not-overlap-with'
RULE = {
    **RULES[0],
    'name': OVERLAPS_WITH,
    'layer': 'polygon',
    'other': 'polygon',
}

# A tool that rejects the fifth feature of the layer that has six, and, when
# that feature is left out, finds no errors
REJECTING = r"""
import json, re, sqlite3
config = open(sys.argv[3], encoding='utf-8').read()
entries = re.findall(
    r'^"([^"]+)" = \{ path = "([^"]*)", layer = "([^"]*)" \}', config, re.M
)
for alias, path, name in entries:
    count = sqlite3.connect(path).execute(
        'select count(*) from "' + name + '"'
    ).fetchone()[0]
    if count == 6:
        error = 'layer ' + alias + ', FID 5, part 0, ring Some(0): '
        error += 'invalid polygon: self-intersection'
        print(json.dumps({'error': error}), file=sys.stderr)
        sys.exit(2)
print(json.dumps({'rules': [{'counts': {}}] * config.count('[[rules]]')}))
"""


@pytest.fixture
def exports(monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """The names of the layers that were written for the tool, one for each
    time."""
    names: list[str] = []
    original = inputs.export_dataset

    def counting(*args: t.Any, **kwargs: t.Any) -> t.Any:
        names.append(args[0].name)
        return original(*args, **kwargs)

    monkeypatch.setattr(inputs, 'export_dataset', counting)
    return names


@pytest.fixture
def rejecting(
    dock: Dock,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
) -> Dock:
    """A dock whose tool rejects a feature of `parcels`, a layer in memory,
    which is checked against `other`, a layer in a file."""
    wait_for(lambda: dock.probing is None)
    configure(
        script_executable(tmp_path, 'import json\n' + REJECTING, rules=[RULE])
    )
    dock.refresh()
    wait_for(lambda: dock.probing is None and dock.available)
    parcels = polygon_layer(SQUARE, *ELSEWHERE[:5], name='parcels')
    QgsProject.instance().addMapLayer(parcels)
    path = tmp_path / 'other.gpkg'
    squares = polygon_layer(SQUARE, *ELSEWHERE[:2], name='other')
    save(squares, path, 'GPKG', 'other')
    other = QgsVectorLayer(f'{path}|layername=other', 'other', 'ogr')
    assert other.isValid()
    QgsProject.instance().addMapLayer(other)
    add(dock, OVERLAPS_WITH, parcels, other)
    return dock


def test_the_layer_that_the_tool_did_not_reject_is_not_written_again(
    rejecting: Dock, exports: list[str]
):
    click_run(rejecting)
    assert exports.count('other') <= 1


def test_the_layer_with_the_rejected_feature_is_written_once_more(
    rejecting: Dock, exports: list[str]
):
    click_run(rejecting)
    assert exports.count('parcels') <= 2


def test_the_rejected_feature_is_reported(rejecting: Dock):
    click_run(rejecting)
    names = [node.text(0) for node in layer_nodes(rejecting)]
    assert 'parcels · 1 invalid' in names
