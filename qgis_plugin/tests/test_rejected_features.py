"""Features that the tool rejects: the message says what stopped the run, and
the ones that cannot be checked are found before the tool runs, not one run at
a time."""

from __future__ import annotations

import collections.abc as c
import math
import re
import typing as t
from pathlib import Path

import pytest
from qgis.core import (
    QgsFeature,
    QgsGeometry,
    QgsPointXY,
    QgsProject,
    QgsVectorLayer,
)

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    calls,
    click_run,
    error_layers,
    layer_nodes,
    message_text,
    polygon_layer,
    save,
    script_executable,
    wait_for,
)
from .test_invalid import ELSEWHERE, FAKE_RULES, OVERLAPS, SQUARE

DANGLES = 'line/must-not-have-dangles'
POINTS_OVERLAP = 'point/must-not-overlap'
# What the tool says about a feature that it cannot take, and about one that
# is not the fault of the feature
INVALID_REASON = 'invalid polygon: self-intersection or closing spike'
OTHER_REASON = 'the geometry kind is not supported by this rule'

# A tool that rejects the fifth feature of an export of six, and, when that
# one is left out, another feature for another reason
REJECTING = f"""
import re, sqlite3
config = open(sys.argv[3], encoding='utf-8').read()
alias = re.search(r'^"([^"]+)" = \\{{', config, re.M).group(1)
path = re.search(r'path = "([^"]*)"', config).group(1)
name = re.search(r'layer = "([^"]*)"', config).group(1)
count = sqlite3.connect(path).execute(
    'select count(*) from "' + name + '"'
).fetchone()[0]
if count == 6:
    error = 'layer ' + alias + ', FID 5, part 0, ring Some(0): {INVALID_REASON}'
else:
    error = 'layer ' + alias + ', FID 3: {OTHER_REASON}'
print(json.dumps({{'error': error}}), file=sys.stderr)
sys.exit(2)
"""


def parcels() -> QgsVectorLayer:
    layer = polygon_layer(SQUARE, *ELSEWHERE[:5], name='parcels')
    QgsProject.instance().addMapLayer(layer)
    return layer


def test_the_message_names_the_error_that_stopped_the_run(
    dock: Dock,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    bar: t.Any,
):
    configure(
        script_executable(
            tmp_path, 'import json\n' + REJECTING, rules=FAKE_RULES
        )
    )
    dock.refresh()
    wait_for(lambda: dock.probing is None and dock.available)
    add(dock, OVERLAPS, parcels())
    click_run(dock)
    text = message_text(bar)
    assert OTHER_REASON in text
    assert 'invalid polygon' not in text


# Points and lines with coordinates that are not numbers


def bad_layer(
    kind: str, good: int, bad: int, directory: Path, value: float = math.nan
) -> QgsVectorLayer:
    """A layer of points or of lines, with `bad` features whose coordinates
    are not finite."""
    geometry = 'Point' if kind == 'point' else 'LineString'
    layer = QgsVectorLayer(f'{geometry}?crs=EPSG:32633', 'bad', 'memory')
    features: list[QgsFeature] = []
    for number in range(good + bad):
        x = float(number * 10)
        y = 0.0 if number < good else value
        feature = QgsFeature()
        if kind == 'point':
            feature.setGeometry(QgsGeometry.fromPointXY(QgsPointXY(x, y)))
        else:
            feature.setGeometry(
                QgsGeometry.fromPolylineXY(
                    [QgsPointXY(x, 0.0), QgsPointXY(x + 5.0, y)]
                )
            )
        features.append(feature)
    layer.dataProvider().addFeatures(features)
    layer.updateExtents()
    # In a file, as QGIS cannot make a source of a memory layer of lines that
    # have coordinates which are not numbers
    path = directory / 'bad.gpkg'
    save(layer, path, 'GPKG', 'bad')
    saved = QgsVectorLayer(f'{path}|layername=bad', 'bad', 'ogr')
    assert saved.isValid()
    QgsProject.instance().addMapLayer(saved)
    return saved


RULE = {'point': POINTS_OVERLAP, 'line': DANGLES}
BAD = 8
TOOL_RULES = [
    {
        'name': name,
        'description': name,
        'arcgis': None,
        'qgis': None,
        'layer': kind,
        'other': None,
    }
    for kind, name in RULE.items()
]

# A tool that checks the coordinates of the features of an export, as the real
# one does: it names the first feature that has a coordinate which is not
# finite, and finds no errors in the others. Every call is written to the log
CHECKING = r"""
import math, re, sqlite3, struct
with open(LOG, 'a') as log:
    log.write(' '.join(sys.argv[1:]) + '\n')
config = open(sys.argv[3], encoding='utf-8').read()
alias = re.search(r'^"([^"]+)" = \{', config, re.M).group(1)
path = re.search(r'path = "([^"]*)"', config).group(1)
name = re.search(r'layer = "([^"]*)"', config).group(1)


def finite(blob):
    envelope = [0, 32, 48, 48, 64][(blob[3] >> 1) & 7]
    wkb = blob[8 + envelope:]
    order = '<' if wkb[0] == 1 else '>'
    kind = struct.unpack(order + 'I', wkb[1:5])[0] % 1000
    if kind == 1:
        values = struct.unpack(order + '2d', wkb[5:21])
    else:
        count = struct.unpack(order + 'I', wkb[5:9])[0]
        values = struct.unpack(
            order + str(2 * count) + 'd', wkb[9:9 + 16 * count]
        )
    return all(math.isfinite(value) for value in values)


rows = sqlite3.connect(path).execute(
    'select geom from "' + name + '" order by fid'
).fetchall()
for number, (blob,) in enumerate(rows, start=1):
    if not finite(blob):
        error = 'layer ' + alias + ', FID ' + str(number)
        error += ': coordinates must be finite'
        print(json.dumps({'error': error}), file=sys.stderr)
        sys.exit(2)
print(json.dumps({'rules': [{'counts': {}}] * config.count('[[rules]]')}))
"""


@pytest.fixture
def counted(
    dock: Dock, tmp_path: Path, configure: c.Callable[[str | Path], None]
) -> Path:
    """A dock whose tool writes each call to a log, which is returned."""
    wait_for(lambda: dock.probing is None)
    log = tmp_path / 'calls'
    body = f'import json\nLOG = {str(log)!r}\n{CHECKING}'
    configure(script_executable(tmp_path, body, rules=TOOL_RULES))
    dock.refresh()
    wait_for(lambda: dock.probing is None and dock.available)
    return log


@pytest.mark.parametrize('kind', ['point', 'line'])
def test_features_with_coordinates_that_are_not_finite_cost_one_run(
    dock: Dock, counted: Path, kind: str, tmp_path: Path
):
    add(dock, RULE[kind], bad_layer(kind, 4, BAD, tmp_path))
    click_run(dock)
    # The run did start, and the tool was not asked again for each feature
    assert 1 <= len(calls(counted, 'validate')) <= 2


@pytest.mark.parametrize('kind', ['point', 'line'])
def test_every_feature_with_coordinates_that_are_not_finite_is_reported(
    dock: Dock, counted: Path, kind: str, tmp_path: Path
):
    add(dock, RULE[kind], bad_layer(kind, 4, BAD, tmp_path))
    click_run(dock)
    (node,) = layer_nodes(dock)
    assert node.text(0) == f'bad · {BAD} invalid'


# The real tool, which says 'coordinates must be finite'


@pytest.mark.parametrize('kind', ['point', 'line'])
@pytest.mark.parametrize('value', [math.inf, math.nan])
def test_the_real_tool_run_completes_with_a_coordinate_that_is_not_finite(
    dock: Dock, tmp_path: Path, kind: str, value: float, bar: t.Any
):
    layer = bad_layer(kind, 4, 1, tmp_path, value)
    add(dock, RULE[kind], layer)
    click_run(dock)
    assert dock.task is None
    if dock.results.rows:
        # The run completed, and reports the feature
        (node,) = layer_nodes(dock)
        assert node.text(0) == 'bad · 1 invalid'
    else:
        # A QGIS that cannot read the layer ends the run with a message that
        # says which layer it is and why
        text = message_text(bar)
        assert 'bad' in text
        assert re.search(r'infinite|non-finite|not finite', text, re.I), text


def finite_vertices(layer: QgsVectorLayer) -> bool:
    return all(
        math.isfinite(vertex.x()) and math.isfinite(vertex.y())
        for feature in layer.getFeatures()
        for vertex in feature.geometry().vertices()
    )


@pytest.mark.parametrize('kind', ['point', 'line'])
@pytest.mark.parametrize('value', [math.inf, math.nan])
def test_no_layer_of_errors_holds_a_coordinate_that_is_not_finite(
    dock: Dock, tmp_path: Path, kind: str, value: float
):
    add(dock, RULE[kind], bad_layer(kind, 4, 1, tmp_path, value))
    click_run(dock)
    assert dock.task is None
    for errors in error_layers():
        assert finite_vertices(errors), errors.name()
