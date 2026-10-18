"""The Processing provider and the rule algorithms, as the toolbox runs them."""

from __future__ import annotations

import collections.abc as c
import json
import subprocess
from pathlib import Path

import pytest
from qgis.core import (
    QgsApplication,
    QgsProcessingAlgorithm,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
    QgsVectorLayer,
)

from topology_rules.algorithms.rule import COUNT
from topology_rules.enums import PROVIDER_ID
from topology_rules.provider import Provider

from .helpers import (
    FAKE_ALGORITHMS,
    fake_executable,
    line_layer,
    point_layer,
    refresh_to_fake_tool,
    save,
)

# Two lines meeting at a node: the free end of each is a dangle
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)


def run(
    name: str, parameters: dict[str, object]
) -> tuple[dict[str, object], bool, QgsProcessingContext]:
    algorithm = QgsApplication.processingRegistry().createAlgorithmById(
        f'{PROVIDER_ID}:{name}'
    )
    assert algorithm is not None
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    results, ok = algorithm.run(parameters, context, QgsProcessingFeedback())
    return results, ok, context


def test_provider_lists_the_rules_of_the_executable(
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    provider: Provider,
):
    configure(fake_executable(tmp_path))
    refresh_to_fake_tool(provider)
    names = {algorithm.name() for algorithm in provider.algorithms()}
    assert names == FAKE_ALGORITHMS
    check = provider.algorithm('check_rules')
    assert check is not None
    # Only the dock runs it
    assert check.flags() & QgsProcessingAlgorithm.Flag.FlagHideFromToolbox
    two_layer = provider.algorithm('point_must_be_covered_by_lines')
    assert two_layer is not None
    assert [p.name() for p in two_layer.parameterDefinitions()] == [
        'INPUT',
        'OTHER',
        'TOLERANCE',
        'ERRORS_POINT',
        'ERRORS_LINE',
        'ERRORS_POLYGON',
    ]


def test_provider_has_no_rules_without_an_executable(
    provider: Provider, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    monkeypatch.setenv('PATH', str(tmp_path))
    provider.refreshAlgorithms()
    assert provider.algorithms() == []


def test_count_matches_the_command_line(
    tmp_path: Path, executable: str, provider: Provider
):
    source = line_layer(*CHAIN, 'LINESTRING(0 5, 10 5)')
    path = tmp_path / 'data.gpkg'
    save(source, path, 'GPKG', 'roads')
    layer = QgsVectorLayer(f'{path}|layername=roads', 'roads', 'ogr')
    QgsProject.instance().addMapLayer(layer)
    results, ok, _ = run('line_must_not_have_dangles', {'INPUT': layer.id()})
    completed = subprocess.run(
        [
            executable,
            *('--json', 'line', 'must-not-have-dangles', str(path)),
            *('--layer', 'roads'),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    report = json.loads(completed.stdout)
    expected = sum(sum(rule['counts'].values()) for rule in report['rules'])
    assert ok
    # The free ends of the chain, and both ends of the separate line
    assert results[COUNT] == expected == 4


def test_two_layer_rule(executable: str, provider: Provider):
    lines = line_layer('LINESTRING(0 0, 10 0)')
    points = point_layer('POINT(5 0)', 'POINT(5 5)')
    QgsProject.instance().addMapLayer(lines)
    QgsProject.instance().addMapLayer(points)
    results, ok, _ = run(
        'point_must_be_covered_by_lines',
        {'INPUT': points.id(), 'OTHER': lines.id()},
    )
    assert ok
    assert results[COUNT] == 1


def test_tool_failure_fails_the_algorithm(
    tmp_path: Path, executable: str, provider: Provider
):
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    _, ok, _ = run(
        'line_must_not_have_dangles',
        {
            'INPUT': layer.id(),
            'ERRORS_POINT': str(tmp_path / 'missing' / 'errors.gpkg'),
        },
    )
    assert not ok


def test_missing_executable_fails_the_algorithm(
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    provider: Provider,
):
    configure(fake_executable(tmp_path))
    refresh_to_fake_tool(provider)
    configure(tmp_path / 'gone')
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    _, ok, _ = run('line_must_not_have_dangles', {'INPUT': layer.id()})
    assert not ok


@pytest.mark.parametrize(
    ('name', 'features'), [('merge_lines', 1), ('explode_lines', 2)]
)
def test_utilities(
    name: str,
    features: int,
    executable: str,
    provider: Provider,
):
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    results, ok, context = run(
        name, {'INPUT': layer.id(), 'OUTPUT': 'memory:output'}
    )
    assert ok
    output = context.getMapLayer(str(results['OUTPUT']))
    assert output is not None
    assert output.featureCount() == features
