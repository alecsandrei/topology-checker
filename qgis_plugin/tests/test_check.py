"""Running several rules at once, through the definition that the tool reads."""

from __future__ import annotations

import collections.abc as c
import json
from pathlib import Path

import pytest
from qgis.core import (
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
    QgsVectorLayer,
)

from topology_rules.algorithms import CheckRulesAlgorithm
from topology_rules.algorithms.check import COUNTS, RULES, SCOPE, hidden_flags
from topology_rules.algorithms.rule import COUNT, OUTPUT, TOLERANCE

from .helpers import (
    both_engines,
    executable_only,
    fake_executable,
    line_layer,
    point_layer,
)

pytestmark = both_engines

DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
# Two lines meeting at a node: the free end of each is a dangle
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


def rules_json(*entries: dict[str, str | None]) -> str:
    return json.dumps(list(entries))


def run(
    parameters: dict[str, object],
) -> tuple[dict[str, object], bool, QgsProcessingFeedback]:
    algorithm = CheckRulesAlgorithm()
    algorithm.initAlgorithm({})
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    feedback = QgsProcessingFeedback()
    results, ok = algorithm.run(parameters, context, feedback)
    return results, ok, feedback


@pytest.fixture
def layers() -> c.Iterator[tuple[QgsVectorLayer, QgsVectorLayer]]:
    lines = line_layer(*CHAIN, 'LINESTRING(0 5, 10 5)', name='roads')
    points = point_layer('POINT(5 0)', 'POINT(5 9)', name='stops')
    project = QgsProject.instance()
    project.addMapLayers([lines, points])
    yield lines, points
    project.removeMapLayers([lines.id(), points.id()])


def test_is_hidden_with_the_parameters_of_the_dock():
    algorithm = CheckRulesAlgorithm()
    algorithm.initAlgorithm({})
    assert [p.name() for p in algorithm.parameterDefinitions()] == [
        RULES,
        SCOPE,
        TOLERANCE,
        OUTPUT,
    ]
    assert algorithm.flags() & hidden_flags() == hidden_flags()


def test_counts_the_errors_of_each_rule(
    tmp_path: Path,
    executable: str,
    layers: tuple[QgsVectorLayer, QgsVectorLayer],
):
    lines, points = layers
    output = tmp_path / 'errors.gpkg'
    results, ok, _ = run(
        {
            RULES: rules_json(
                {'rule': DANGLES, 'layer': lines.id()},
                {'rule': COVERED, 'layer': points.id(), 'other': lines.id()},
            ),
            OUTPUT: str(output),
        }
    )
    assert ok
    # The ends of the chain and both ends of the separate line, and one stop
    assert json.loads(str(results[COUNTS])) == [4, 1]
    assert results[COUNT] == 5
    errors = QgsVectorLayer(f'{output}|layername=errors_point', 'e', 'ogr')
    assert sorted(f['rule'] for f in errors.getFeatures()) == [
        *[DANGLES] * 4,
        COVERED,
    ]


def test_a_rule_without_errors_counts_zero(tmp_path: Path, executable: str):
    ring = line_layer('LINESTRING(0 0, 10 0, 10 10, 0 0)', name='ring')
    QgsProject.instance().addMapLayer(ring)
    results, ok, _ = run(
        {
            RULES: rules_json({'rule': DANGLES, 'layer': ring.id()}),
            TOLERANCE: 0.5,
            OUTPUT: str(tmp_path / 'errors.gpkg'),
        }
    )
    QgsProject.instance().removeMapLayer(ring.id())
    assert ok
    assert json.loads(str(results[COUNTS])) == [0]
    assert results[COUNT] == 0


@pytest.mark.parametrize(
    ('rules', 'message'),
    [
        ('not json', 'Invalid rules'),
        ('[]', 'Invalid rules'),
        ('[{"rule": "x"}]', 'Invalid rules'),
        ('[{"rule": "line/nope", "layer": "a"}]', 'Unknown rule'),
        (f'[{{"rule": "{DANGLES}", "layer": "gone"}}]', 'The layer is gone'),
        (
            f'[{{"rule": "{DANGLES}", "layer": "a", "other": "b"}}]',
            'Wrong number of layers',
        ),
        (f'[{{"rule": "{COVERED}", "layer": "a"}}]', 'Wrong number of layers'),
    ],
)
def test_bad_rules_fail_the_algorithm(
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    rules: str,
    message: str,
):
    configure(fake_executable(tmp_path))
    _, ok, feedback = run({RULES: rules, OUTPUT: str(tmp_path / 'errors.gpkg')})
    assert not ok
    assert message in feedback.textLog()


@executable_only
def test_missing_executable_fails_the_algorithm(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setenv('PATH', str(tmp_path))
    _, ok, feedback = run(
        {
            RULES: rules_json({'rule': DANGLES, 'layer': 'a'}),
            OUTPUT: str(tmp_path / 'errors.gpkg'),
        }
    )
    assert not ok
    assert 'topology-checker was not found' in feedback.textLog()
