"""The Processing algorithm that runs a topology.toml."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest
from qgis.core import (
    QgsApplication,
    QgsProcessingAlgorithm,
    QgsProcessingParameterDefinition,
    QgsProcessingParameterFeatureSink,
    QgsProcessingParameterFile,
    QgsProcessingParameterNumber,
)

from topology_rules.algorithms.rule import COUNT
from topology_rules.enums import PROVIDER_ID
from topology_rules.provider import Provider

from .helpers import (
    cli_counts,
    errors_layer,
    run_algorithm,
    saved_layer,
    saved_points,
    write_toml,
)

DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
ALGORITHM = f'{PROVIDER_ID}:check_definition'


@pytest.fixture(autouse=True)
def ready(executable: str, provider: Provider) -> None:
    """The real executable and the provider, as in a QGIS session."""


@pytest.fixture
def definition(tmp_path: Path) -> Path:
    """Dangles on roads and stops that must lie on them: 2 and 1 errors."""
    data = tmp_path / 'data'
    data.mkdir()
    saved_layer(data, CHAIN, 'roads')
    saved_points(data, ['POINT(5 0)', 'POINT(5 5)'])
    return write_toml(
        tmp_path / 'topology.toml',
        {
            'roads': ('data/roads.gpkg', 'roads'),
            'stops': ('data/stops.gpkg', 'stops'),
        },
        [(DANGLES, 'roads', None), (COVERED, 'stops', 'roads')],
    )


def algorithm() -> QgsProcessingAlgorithm:
    found = QgsApplication.processingRegistry().createAlgorithmById(ALGORITHM)
    assert found is not None
    return found


def check(definition: object, **parameters: object):
    return run_algorithm(
        'check_definition', {'DEFINITION': str(definition), **parameters}
    )


def test_the_algorithm_is_named():
    alg = algorithm()
    assert alg.displayName() == 'Check topology definition'


def test_the_algorithm_takes_a_toml_file():
    alg = algorithm()
    parameter = alg.parameterDefinition('DEFINITION')
    assert isinstance(parameter, QgsProcessingParameterFile)
    assert 'toml' in parameter.fileFilter().lower() or (
        parameter.extension() == 'toml'
    )


def test_the_tolerance_is_an_optional_number():
    alg = algorithm()
    parameter = alg.parameterDefinition('TOLERANCE')
    assert isinstance(parameter, QgsProcessingParameterNumber)
    assert (
        parameter.flags() & QgsProcessingParameterDefinition.Flag.FlagOptional
    )


@pytest.mark.parametrize(
    'name', ['ERRORS_POINT', 'ERRORS_LINE', 'ERRORS_POLYGON']
)
def test_the_errors_are_feature_sinks(name: str):
    alg = algorithm()
    assert isinstance(
        alg.parameterDefinition(name), QgsProcessingParameterFeatureSink
    )


def test_the_algorithm_has_a_count():
    alg = algorithm()
    names = [output.name() for output in alg.outputDefinitions()]
    assert COUNT in names


def test_the_algorithm_is_in_the_toolbox(provider: Provider):
    alg = algorithm()
    flags = alg.flags()
    assert not flags & QgsProcessingAlgorithm.Flag.FlagHideFromToolbox
    assert provider.algorithm('check_definition') is not None


def test_the_algorithm_is_in_the_modeler():
    alg = algorithm()
    assert not alg.flags() & QgsProcessingAlgorithm.Flag.FlagHideFromModeler


def test_the_count_is_the_count_of_the_command_line(
    definition: Path, executable: str
):
    ran = check(definition)
    assert ran.ok, ran.log
    assert ran.results[COUNT] == sum(cli_counts(executable, definition)) == 3


def test_the_errors_are_in_the_sinks_by_geometry(definition: Path):
    ran = check(definition)
    assert errors_layer(ran, 'ERRORS_POINT').featureCount() == 3
    assert errors_layer(ran, 'ERRORS_LINE').featureCount() == 0
    assert errors_layer(ran, 'ERRORS_POLYGON').featureCount() == 0


def test_the_origin_layers_are_the_aliases_of_the_file(definition: Path):
    ran = check(definition)
    layers = {
        feature['origin_layer']
        for feature in errors_layer(ran, 'ERRORS_POINT').getFeatures()
    }
    assert layers == {'roads', 'stops'}


def test_the_rule_of_each_error_is_named(definition: Path):
    ran = check(definition)
    rules = {
        feature['rule']
        for feature in errors_layer(ran, 'ERRORS_POINT').getFeatures()
    }
    assert rules == {DANGLES, COVERED}


def test_a_tolerance_replaces_the_one_of_the_file(
    tmp_path: Path, executable: str
):
    data = tmp_path / 'data'
    data.mkdir()
    saved_layer(
        data, ('LINESTRING(0 0, 10 0)', 'LINESTRING(10.4 0, 20 0)'), 'gap'
    )
    file = write_toml(
        tmp_path / 'topology.toml',
        {'gap': ('data/gap.gpkg', 'gap')},
        [(DANGLES, 'gap', None)],
    )
    assert check(file).results[COUNT] == 4
    ran = check(file, TOLERANCE=1.0)
    assert ran.results[COUNT] == 2 == sum(cli_counts(executable, file, 1.0))


def test_the_tolerance_of_the_file_is_used(tmp_path: Path):
    data = tmp_path / 'data'
    data.mkdir()
    saved_layer(
        data, ('LINESTRING(0 0, 10 0)', 'LINESTRING(10.4 0, 20 0)'), 'gap'
    )
    file = write_toml(
        tmp_path / 'topology.toml',
        {'gap': ('data/gap.gpkg', 'gap')},
        [(DANGLES, 'gap', None)],
        tolerance=1.0,
    )
    assert check(file).results[COUNT] == 2


def test_relative_paths_are_the_ones_of_the_file(
    definition: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    elsewhere = tmp_path / 'elsewhere'
    elsewhere.mkdir()
    monkeypatch.chdir(elsewhere)
    assert check(definition).ok


def test_a_file_without_errors_has_empty_sinks(tmp_path: Path):
    data = tmp_path / 'data'
    data.mkdir()
    saved_layer(data, ('LINESTRING(0 0, 10 0, 10 10, 0 0)',), 'ring')
    file = write_toml(
        tmp_path / 'topology.toml',
        {'ring': ('data/ring.gpkg', 'ring')},
        [(DANGLES, 'ring', None)],
    )
    ran = check(file)
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 0
    assert errors_layer(ran, 'ERRORS_POINT').featureCount() == 0
    assert ran.context.layersToLoadOnCompletion() == {}


def cli_error(path: Path, executable: str) -> str:
    done = subprocess.run(
        [executable, '--json', 'validate', str(path)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 2
    return str(json.loads(done.stderr.strip().splitlines()[-1])['error'])


@pytest.mark.parametrize('kind', ['syntax', 'rule', 'key', 'data'])
def test_a_bad_definition_fails_with_the_message_of_the_tool(
    tmp_path: Path, executable: str, kind: str
):
    path = tmp_path / 'bad.toml'
    layers = {'roads': ('gone.gpkg', 'roads')}
    if kind == 'syntax':
        path.write_text('this is = not [ toml', encoding='utf-8')
    elif kind == 'rule':
        write_toml(path, layers, [('line/must-not-exist', 'roads', None)])
    elif kind == 'key':
        path.write_text('bogus = 1\n', encoding='utf-8')
    else:
        write_toml(path, layers, [(DANGLES, 'roads', None)])
    ran = check(path)
    assert not ran.ok
    assert cli_error(path, executable) in ran.log


def test_a_missing_definition_fails(tmp_path: Path):
    ran = check(tmp_path / 'nothing.toml')
    assert not ran.ok
    assert ran.log.strip()


@pytest.mark.parametrize(
    'sink', ['ERRORS_POINT', 'ERRORS_LINE', 'ERRORS_POLYGON']
)
@pytest.mark.parametrize('name', ['roads', 'stops'])
def test_an_output_on_a_data_file_of_the_definition_is_refused(
    definition: Path, name: str, sink: str
):
    path = definition.parent / 'data' / f'{name}.gpkg'
    before = path.read_bytes()
    ran = check(definition, **{sink: str(path)})
    assert not ran.ok
    assert 'input' in ran.log.lower()
    assert path.read_bytes() == before
