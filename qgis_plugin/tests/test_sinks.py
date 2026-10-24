"""The errors of a rule come out as three feature sinks."""

from __future__ import annotations

from pathlib import Path

import pytest
from qgis.core import (
    QgsApplication,
    QgsProcessingOutputNumber,
    QgsProcessingParameterFeatureSink,
    QgsProject,
    QgsVectorLayer,
)

from topology_rules.algorithms.rule import COUNT
from topology_rules.core.engine import Engine
from topology_rules.enums import PROVIDER_ID

from .helpers import (
    errors_layer,
    line_layer,
    point_layer,
    run_algorithm,
    saved_layer,
)

DANGLES = 'line_must_not_have_dangles'
SINKS = ('ERRORS_POINT', 'ERRORS_LINE', 'ERRORS_POLYGON')
FIELDS = [
    'violation_id',
    'rule',
    'error_type',
    'origin_layer',
    'origin_fid',
    'origin_part',
    'destination_layer',
    'destination_fid',
    'destination_part',
]
# Two lines meeting at a node: the free end of each is a dangle
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)


@pytest.fixture(autouse=True)
def ready(run_engine: Engine, executable: str, provider: object) -> None:
    """The real executable and the provider, as in a QGIS session, and
    each of the two engines in turn."""


def project_layer(wkts: tuple[str, ...]) -> QgsVectorLayer:
    layer = line_layer(*wkts)
    QgsProject.instance().addMapLayer(layer)
    return layer


def test_a_rule_has_a_sink_for_each_geometry_of_errors():
    algorithm = QgsApplication.processingRegistry().createAlgorithmById(
        f'{PROVIDER_ID}:{DANGLES}'
    )
    assert algorithm is not None
    for name in SINKS:
        definition = algorithm.parameterDefinition(name)
        assert isinstance(definition, QgsProcessingParameterFeatureSink)
    assert algorithm.parameterDefinition('OUTPUT') is None
    outputs = {
        output.name(): output for output in algorithm.outputDefinitions()
    }
    assert isinstance(outputs[COUNT], QgsProcessingOutputNumber)


def test_sinks_are_temporary_layers_by_default():
    ran = run_algorithm(DANGLES, {'INPUT': project_layer(CHAIN).id()})
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 2
    assert errors_layer(ran, 'ERRORS_POINT').featureCount() == 2


def test_the_fields_mirror_the_columns_of_the_tool():
    ran = run_algorithm(DANGLES, {'INPUT': project_layer(CHAIN).id()})
    layer = errors_layer(ran, 'ERRORS_POINT')
    assert layer.fields().names() == FIELDS


def test_the_errors_have_the_crs_of_the_input():
    layer = project_layer(CHAIN)
    ran = run_algorithm(DANGLES, {'INPUT': layer.id()})
    assert errors_layer(ran, 'ERRORS_POINT').crs() == layer.crs()


def test_the_attributes_say_what_was_found():
    ran = run_algorithm(DANGLES, {'INPUT': project_layer(CHAIN).id()})
    feature = next(errors_layer(ran, 'ERRORS_POINT').getFeatures())
    assert feature['error_type'] == 'dangle'
    assert feature['rule'] == 'line/must-not-have-dangles'


def test_all_three_sinks_exist_when_there_are_no_errors():
    ran = run_algorithm(DANGLES, {'INPUT': project_layer(RING).id()})
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 0
    for name in SINKS:
        assert errors_layer(ran, name).featureCount() == 0


def test_only_sinks_with_errors_are_loaded_into_the_project():
    ran = run_algorithm(DANGLES, {'INPUT': project_layer(CHAIN).id()})
    assert len(ran.context.layersToLoadOnCompletion()) == 1


def test_nothing_is_loaded_when_there_are_no_errors():
    ran = run_algorithm(DANGLES, {'INPUT': project_layer(RING).id()})
    assert ran.context.layersToLoadOnCompletion() == {}


@pytest.mark.parametrize(
    'name', ['errors.gpkg', 'errors.shp', 'errors.geojson']
)
def test_a_sink_can_be_a_file(tmp_path: Path, name: str):
    path = tmp_path / name
    ran = run_algorithm(
        DANGLES,
        {'INPUT': project_layer(CHAIN).id(), 'ERRORS_POINT': str(path)},
    )
    assert ran.ok, ran.log
    assert path.exists()
    assert QgsVectorLayer(str(path), 'e', 'ogr').featureCount() == 2


def test_a_sink_can_be_a_memory_layer():
    ran = run_algorithm(
        DANGLES,
        {
            'INPUT': project_layer(CHAIN).id(),
            'ERRORS_POINT': 'memory:errors',
        },
    )
    assert ran.ok, ran.log
    assert errors_layer(ran, 'ERRORS_POINT').featureCount() == 2


def test_a_sink_on_the_second_input_file_is_refused(tmp_path: Path):
    lines = saved_layer(tmp_path, CHAIN)
    QgsProject.instance().addMapLayer(lines)
    points = point_layer('POINT(5 0)')
    QgsProject.instance().addMapLayer(points)
    path = Path(lines.source().split('|')[0])
    before = path.read_bytes()
    ran = run_algorithm(
        'point_must_be_covered_by_lines',
        {
            'INPUT': points.id(),
            'OTHER': lines.id(),
            'ERRORS_POINT': str(path),
        },
    )
    assert not ran.ok
    assert path.read_bytes() == before
