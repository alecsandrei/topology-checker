"""The layers a rule reads: feature sources, their ids and their CRS."""

from __future__ import annotations

from pathlib import Path

import pytest
from qgis.core import (
    QgsApplication,
    QgsCoordinateReferenceSystem,
    QgsCoordinateTransform,
    QgsFeature,
    QgsGeometry,
    QgsProcessingFeatureSourceDefinition,
    QgsProcessingParameterDistance,
    QgsProject,
    QgsVectorLayer,
)

from topology_rules.algorithms.rule import COUNT
from topology_rules.core.engine import Engine
from topology_rules.enums import PROVIDER_ID

from .helpers import (
    Ran,
    add_features,
    errors_layer,
    line_layer,
    run_algorithm,
    saved_layer,
)
from .test_inputs import ORIGINS

DANGLES = 'line_must_not_have_dangles'
COVERED = 'point_must_be_covered_by_lines'
# Two lines meeting at a node: the free end of each is a dangle
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
# A line far from the chain, with two more dangles
FAR = 'LINESTRING(0 50, 10 50)'


@pytest.fixture(autouse=True)
def ready(run_engine: Engine, executable: str, provider: object) -> None:
    """The real executable and the provider, as in a QGIS session, and
    each of the two engines in turn."""


def project_layer(tmp_path: Path, *wkts: str) -> QgsVectorLayer:
    layer = saved_layer(tmp_path, wkts)
    QgsProject.instance().addMapLayer(layer)
    return layer


def assert_errors_point_at_features(
    layer: QgsVectorLayer, source: object, count: int
) -> None:
    """Runs the rule and finds every error's feature by the id it names."""
    ran = run_algorithm(DANGLES, {'INPUT': source})
    assert ran.ok, ran.log
    errors = errors_layer(ran, 'ERRORS_POINT')
    assert errors.featureCount() == count
    for error in errors.getFeatures():
        feature = layer.getFeature(int(error['origin_fid']))
        assert feature.isValid()
        assert feature.geometry().intersects(error.geometry())


def test_selected_features_only(tmp_path: Path):
    layer = project_layer(tmp_path, *CHAIN, FAR)
    layer.selectByIds([1, 2])
    everything = run_algorithm(DANGLES, {'INPUT': layer.id()})
    selected = run_algorithm(
        DANGLES,
        {'INPUT': QgsProcessingFeatureSourceDefinition(layer.id(), True)},
    )
    assert everything.results[COUNT] == 4
    assert selected.results[COUNT] == 2


def test_selected_features_only_without_a_selection_checks_nothing(
    tmp_path: Path,
):
    layer = project_layer(tmp_path, *CHAIN)
    ran = run_algorithm(
        DANGLES,
        {'INPUT': QgsProcessingFeatureSourceDefinition(layer.id(), True)},
    )
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 0


def test_a_feature_limit(tmp_path: Path):
    layer = project_layer(tmp_path, *CHAIN, FAR)
    ran = run_algorithm(
        DANGLES,
        {'INPUT': QgsProcessingFeatureSourceDefinition(layer.id(), False, 2)},
    )
    assert ran.results[COUNT] == 2


@pytest.mark.parametrize('origin', ORIGINS)
def test_error_ids_are_feature_ids_of_the_layer(origin: str, tmp_path: Path):
    layer = ORIGINS[origin](tmp_path)
    QgsProject.instance().addMapLayer(layer)
    assert_errors_point_at_features(layer, layer.id(), 2)


def test_error_ids_of_a_filtered_layer(tmp_path: Path):
    layer = project_layer(tmp_path, FAR, *CHAIN)
    layer.setSubsetString('fid > 1')
    assert_errors_point_at_features(layer, layer.id(), 2)


def test_error_ids_of_the_selected_features(tmp_path: Path):
    layer = project_layer(tmp_path, FAR, *CHAIN)
    layer.selectByIds([2, 3])
    source = QgsProcessingFeatureSourceDefinition(layer.id(), True)
    assert_errors_point_at_features(layer, source, 2)


def origin_ids(ran: Ran) -> list[int]:
    """The ids of the features of the point errors, in the order of the
    layer."""
    errors = errors_layer(ran, 'ERRORS_POINT')
    return [int(error['origin_fid']) for error in errors.getFeatures()]


def test_the_errors_of_a_selection_are_in_the_order_of_the_ids():
    """QGIS gives the selection of a memory layer in an order of its own,
    which changes from one session to the next."""
    layer = line_layer(
        *(f'LINESTRING({x} 0, {x} 5)' for x in range(0, 400, 10))
    )
    QgsProject.instance().addMapLayer(layer)
    layer.selectByIds(list(range(5, 35)))
    ran = run_algorithm(
        DANGLES,
        {'INPUT': QgsProcessingFeatureSourceDefinition(layer.id(), True)},
    )
    assert ran.ok, ran.log
    assert origin_ids(ran) == sorted(2 * list(range(5, 35)))


def test_the_errors_of_new_features_are_in_the_order_of_the_ids(
    tmp_path: Path,
):
    layer = project_layer(tmp_path, *CHAIN)
    assert layer.startEditing()
    try:
        for y in (100, 200, 300):
            feature = QgsFeature(layer.fields())
            feature.setGeometry(
                QgsGeometry.fromWkt(f'LINESTRING(0 {y}, 10 {y})')
            )
            assert layer.addFeature(feature)
        ran = run_algorithm(DANGLES, {'INPUT': layer.id()})
    finally:
        layer.rollBack()
    assert ran.ok, ran.log
    found = origin_ids(ran)
    assert found == sorted(found)
    # Two dangles of the chain, and two of each new line
    assert len(found) == 8


def test_error_ids_of_an_edited_layer(tmp_path: Path):
    layer = project_layer(tmp_path, *CHAIN)
    assert layer.startEditing()
    feature = QgsFeature(layer.fields())
    feature.setGeometry(QgsGeometry.fromWkt(FAR))
    assert layer.addFeature(feature)
    try:
        assert_errors_point_at_features(layer, layer.id(), 4)
    finally:
        layer.rollBack()


def test_error_ids_of_a_layer_with_a_hole_in_its_ids(tmp_path: Path):
    layer = project_layer(tmp_path, FAR, *CHAIN)
    assert layer.startEditing()
    assert layer.deleteFeature(1)
    assert layer.commitChanges()
    assert_errors_point_at_features(layer, layer.id(), 2)


def crs_pair(
    wkt_points: tuple[str, ...], wkt_line: str, line_crs: str
) -> tuple[QgsVectorLayer, QgsVectorLayer]:
    """Points in EPSG:4326 and a line, given in EPSG:4326, in another CRS."""
    points = QgsVectorLayer('Point?crs=EPSG:4326', 'points', 'memory')
    add_features(points, wkt_points)
    geometry = QgsGeometry.fromWkt(wkt_line)
    source = QgsCoordinateReferenceSystem('EPSG:4326')
    target = QgsCoordinateReferenceSystem(line_crs)
    geometry.transform(
        QgsCoordinateTransform(source, target, QgsProject.instance())
    )
    lines = QgsVectorLayer(f'LineString?crs={line_crs}', 'lines', 'memory')
    add_features(lines, [geometry.asWkt()])
    for layer in (points, lines):
        QgsProject.instance().addMapLayer(layer)
    return points, lines


POINTS = ('POINT(0.5 0)', 'POINT(0.5 0.5)')
LINE = 'LINESTRING(0 0, 1 0)'


def covered(points: QgsVectorLayer, lines: QgsVectorLayer):
    return run_algorithm(COVERED, {'INPUT': points.id(), 'OTHER': lines.id()})


@pytest.mark.parametrize('other_crs', ['EPSG:3857', 'EPSG:32631'])
def test_the_other_layer_is_reprojected_to_the_crs_of_the_input(
    other_crs: str,
):
    same = covered(*crs_pair(POINTS, LINE, 'EPSG:4326'))
    mixed = covered(*crs_pair(POINTS, LINE, other_crs))
    assert same.ok, same.log
    assert mixed.ok, mixed.log
    assert same.results[COUNT] == mixed.results[COUNT] == 1


def test_the_log_mentions_the_reprojection():
    ran = covered(*crs_pair(POINTS, LINE, 'EPSG:3857'))
    assert 'reproject' in ran.log.lower()


def test_the_log_is_silent_when_the_crs_agree():
    ran = covered(*crs_pair(POINTS, LINE, 'EPSG:4326'))
    assert 'reproject' not in ran.log.lower()


def test_the_errors_have_the_crs_of_the_first_layer():
    points, lines = crs_pair(POINTS, LINE, 'EPSG:3857')
    ran = covered(points, lines)
    assert errors_layer(ran, 'ERRORS_POINT').crs() == points.crs()


def test_the_tolerance_is_a_distance_in_the_units_of_the_input():
    algorithm = QgsApplication.processingRegistry().createAlgorithmById(
        f'{PROVIDER_ID}:{DANGLES}'
    )
    assert algorithm is not None
    tolerance = algorithm.parameterDefinition('TOLERANCE')
    assert isinstance(tolerance, QgsProcessingParameterDistance)
    assert tolerance.parentParameterName() == 'INPUT'


def test_an_empty_tolerance_is_the_default_of_the_crs(tmp_path: Path):
    layer = project_layer(tmp_path, *CHAIN)
    ran = run_algorithm(DANGLES, {'INPUT': layer.id(), 'TOLERANCE': None})
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 2


def test_the_tolerance_is_in_the_units_of_the_input(tmp_path: Path):
    # A gap of 0.0004 degrees is closed by a tolerance of 0.001 degrees
    layer = QgsVectorLayer('LineString?crs=EPSG:4326', 'degrees', 'memory')
    add_features(layer, ['LINESTRING(0 0, 1 0)', 'LINESTRING(1.0004 0, 2 0)'])
    QgsProject.instance().addMapLayer(layer)
    counts = [
        run_algorithm(
            DANGLES, {'INPUT': layer.id(), 'TOLERANCE': tolerance}
        ).results[COUNT]
        for tolerance in (0, 0.001)
    ]
    assert counts == [4, 2]
