"""The parameters a user can give to an algorithm, sensible or not."""

from __future__ import annotations

import os
from pathlib import Path

import pytest
from qgis.core import (
    NULL,
    QgsApplication,
    QgsProcessingContext,
    QgsProject,
    QgsVectorLayer,
)

from topology_rules.algorithms.rule import COUNT, OUTPUT
from topology_rules.core.engine import Engine
from topology_rules.enums import PROVIDER_ID

from .helpers import (
    line_layer,
    point_layer,
    run_algorithm,
    saved_layer,
)

DANGLES = 'line_must_not_have_dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


@pytest.fixture(autouse=True)
def ready(run_engine: Engine, executable: str, provider: object) -> None:
    """The real executable and the provider, as in a QGIS session, and
    each of the two engines in turn."""


def memory_chain() -> QgsVectorLayer:
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    return layer


@pytest.mark.parametrize(
    'tolerance', [None, NULL, '', 0, 0.0, 0.001, '0.5', 5, 1e-12]
)
def test_tolerances_that_mean_something(tolerance: object):
    layer = memory_chain()
    parameters = {'INPUT': layer.id()}
    if tolerance is not None:
        parameters['TOLERANCE'] = tolerance
    ran = run_algorithm(DANGLES, parameters)
    assert ran.ok, ran.log
    # The chain is two lines meeting exactly; no tolerance joins the far ends
    assert ran.results[COUNT] == 2


@pytest.mark.parametrize('tolerance', ['abc', -1, -0.5, float('nan')])
def test_tolerances_that_mean_nothing_are_not_accepted(tolerance: object):
    algorithm = QgsApplication.processingRegistry().createAlgorithmById(
        f'{PROVIDER_ID}:{DANGLES}'
    )
    assert algorithm is not None
    ok, _ = algorithm.checkParameterValues(
        {'INPUT': 'x', 'TOLERANCE': tolerance}, QgsProcessingContext()
    )
    assert not ok


@pytest.mark.parametrize('tolerance', [-1, -0.5, float('nan')])
def test_tolerances_that_mean_nothing_fail_with_a_message(tolerance: object):
    layer = memory_chain()
    ran = run_algorithm(DANGLES, {'INPUT': layer.id(), 'TOLERANCE': tolerance})
    assert not ran.ok
    assert ran.log.strip()


def test_the_tolerance_joins_a_small_gap():
    layer = line_layer('LINESTRING(0 0, 10 0)', 'LINESTRING(10.4 0, 20 0)')
    QgsProject.instance().addMapLayer(layer)
    counts = []
    for tolerance in (0, 1):
        ran = run_algorithm(
            DANGLES, {'INPUT': layer.id(), 'TOLERANCE': tolerance}
        )
        assert ran.ok, ran.log
        counts.append(ran.results[COUNT])
    assert counts == [4, 2]


@pytest.mark.skipif(
    os.name == 'nt' or os.geteuid() == 0,
    reason='needs a folder that refuses, which root can write to',
)
def test_a_sink_in_a_folder_that_cannot_be_written(tmp_path: Path):
    layer = memory_chain()
    folder = tmp_path / 'locked'
    folder.mkdir()
    folder.chmod(0o500)
    try:
        ran = run_algorithm(
            DANGLES,
            {'INPUT': layer.id(), 'ERRORS_POINT': str(folder / 'e.gpkg')},
        )
    finally:
        folder.chmod(0o700)
    assert not ran.ok
    assert ran.log.strip()


def test_a_sink_over_an_existing_file_replaces_it(tmp_path: Path):
    layer = memory_chain()
    output = tmp_path / 'e.gpkg'
    output.write_text('not a geopackage')
    ran = run_algorithm(
        DANGLES, {'INPUT': layer.id(), 'ERRORS_POINT': str(output)}
    )
    assert ran.ok, ran.log
    assert QgsVectorLayer(str(output), 'e', 'ogr').featureCount() == 2


@pytest.mark.parametrize('driver', ['GPKG', 'ESRI Shapefile', 'GeoJSON'])
def test_a_sink_on_the_input_file_is_refused(tmp_path: Path, driver: str):
    layer = saved_layer(tmp_path, CHAIN, driver=driver)
    QgsProject.instance().addMapLayer(layer)
    path = Path(layer.source().split('|')[0])
    before = path.read_bytes()
    ran = run_algorithm(
        DANGLES, {'INPUT': layer.id(), 'ERRORS_POINT': str(path)}
    )
    assert not ran.ok
    assert 'input' in ran.log.lower()
    assert path.read_bytes() == before


def test_a_second_run_into_a_sink_whose_layer_is_loaded(tmp_path: Path):
    layer = memory_chain()
    output = tmp_path / 'e.gpkg'
    first = run_algorithm(
        DANGLES, {'INPUT': layer.id(), 'ERRORS_POINT': str(output)}
    )
    assert first.ok, first.log
    # What Processing does after a run: the error layers join the project
    for uri, details in first.context.layersToLoadOnCompletion().items():
        loaded = QgsVectorLayer(uri, details.name, 'ogr')
        assert loaded.isValid()
        QgsProject.instance().addMapLayer(loaded)
    second = run_algorithm(
        DANGLES, {'INPUT': layer.id(), 'ERRORS_POINT': str(output)}
    )
    assert second.ok, second.log
    assert second.results[COUNT] == 2


def test_the_errors_of_a_run_as_the_input_of_the_next(tmp_path: Path):
    layer = memory_chain()
    output = tmp_path / 'e.gpkg'
    first = run_algorithm(
        DANGLES, {'INPUT': layer.id(), 'ERRORS_POINT': str(output)}
    )
    assert first.ok, first.log
    errors = QgsVectorLayer(str(output), 'e', 'ogr')
    assert errors.isValid()
    QgsProject.instance().addMapLayer(errors)
    before = output.read_bytes()
    ran = run_algorithm(
        'point_must_not_overlap',
        {'INPUT': errors.id(), 'ERRORS_POINT': str(output)},
    )
    assert not ran.ok
    assert output.read_bytes() == before


def test_input_as_a_path(tmp_path: Path):
    layer = saved_layer(tmp_path, CHAIN)
    path = layer.source().split('|')[0]
    ran = run_algorithm(DANGLES, {'INPUT': path})
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 2


@pytest.mark.parametrize('value', ['no such layer', '', None, 42])
def test_input_that_is_no_layer(value: object):
    ran = run_algorithm(DANGLES, {'INPUT': value})
    assert not ran.ok


def test_second_layer_is_required_by_a_two_layer_rule():
    points = point_layer('POINT(5 0)')
    QgsProject.instance().addMapLayer(points)
    ran = run_algorithm(
        'point_must_be_covered_by_lines', {'INPUT': points.id()}
    )
    assert not ran.ok


def test_both_inputs_are_the_same_layer():
    layer = memory_chain()
    ran = run_algorithm(
        'line_must_not_overlap_with',
        {'INPUT': layer.id(), 'OTHER': layer.id()},
    )
    assert ran.ok or ran.log.strip()


@pytest.mark.parametrize('name', ['merge_lines', 'explode_lines'])
@pytest.mark.parametrize(
    'output', ['memory:output', 'TEMPORARY_OUTPUT', 'file.gpkg', 'file.shp']
)
def test_utility_outputs(tmp_path: Path, name: str, output: str):
    layer = memory_chain()
    if output.startswith('file'):
        output = str(tmp_path / output)
    ran = run_algorithm(name, {'INPUT': layer.id(), 'OUTPUT': output})
    assert ran.ok, ran.log
    assert ran.results[OUTPUT]


@pytest.mark.parametrize('name', ['merge_lines', 'explode_lines'])
@pytest.mark.parametrize('driver', ['GPKG', 'ESRI Shapefile'])
def test_a_utility_output_on_the_input_file_is_refused(
    tmp_path: Path, name: str, driver: str
):
    layer = saved_layer(tmp_path, CHAIN, driver=driver)
    QgsProject.instance().addMapLayer(layer)
    path = Path(layer.source().split('|')[0])
    before = path.read_bytes()
    ran = run_algorithm(name, {'INPUT': layer.id(), 'OUTPUT': str(path)})
    assert not ran.ok
    assert 'input' in ran.log.lower()
    assert path.read_bytes() == before


def test_utility_on_a_polygon_layer_is_refused():
    layer = QgsVectorLayer('Polygon?crs=EPSG:32633', 'p', 'memory')
    QgsProject.instance().addMapLayer(layer)
    ran = run_algorithm(
        'merge_lines', {'INPUT': layer.id(), 'OUTPUT': 'memory:output'}
    )
    assert not ran.ok
