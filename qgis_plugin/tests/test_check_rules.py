"""What the hidden algorithm of the dock refuses, and how it fails."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from qgis.core import QgsFeature, QgsGeometry, QgsProject, QgsVectorLayer

from topology_rules.algorithms.rule import COUNT
from topology_rules.core.engine import Engine

from .helpers import (
    files_stay_open,
    line_layer,
    point_layer,
    remove_files,
    run_algorithm,
    saved_layer,
)

DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
FAR = 'LINESTRING(0 50, 10 50)'


@pytest.fixture(autouse=True)
def ready(run_engine: Engine, executable: str, provider: object) -> None:
    """The real executable and the provider, as in a QGIS session, and
    each of the two engines in turn."""


def check(tmp_path: Path, *entries: dict[str, object], output: object = None):
    return run_algorithm(
        'check_rules',
        {
            'RULES': json.dumps(list(entries)),
            'OUTPUT': str(output or tmp_path / 'errors.gpkg'),
        },
    )


def entry(
    rule: str, layer: QgsVectorLayer, other: QgsVectorLayer | None = None
):
    return {
        'rule': rule,
        'layer': layer.id(),
        'other': None if other is None else other.id(),
    }


def saved(
    tmp_path: Path, *wkts: str, name: str = 'roads', driver: str = 'GPKG'
) -> QgsVectorLayer:
    layer = saved_layer(tmp_path, wkts, name, driver)
    QgsProject.instance().addMapLayer(layer)
    return layer


def test_a_layer_of_the_wrong_kind_is_refused(tmp_path: Path):
    points = point_layer('POINT(0 0)')
    QgsProject.instance().addMapLayer(points)
    ran = check(tmp_path, entry(DANGLES, points))
    assert not ran.ok
    assert 'line layer' in ran.log


def test_a_second_layer_of_the_wrong_kind_is_refused(tmp_path: Path):
    points = point_layer('POINT(0 0)')
    QgsProject.instance().addMapLayer(points)
    ran = check(tmp_path, entry(COVERED, points, points))
    assert not ran.ok


def test_a_layer_that_is_gone_is_refused(tmp_path: Path):
    layer = saved(tmp_path, *CHAIN)
    description = entry(DANGLES, layer)
    QgsProject.instance().removeMapLayer(layer.id())
    ran = check(tmp_path, description)
    assert not ran.ok
    assert 'gone' in ran.log


@files_stay_open
def test_a_layer_whose_file_vanished_is_refused(tmp_path: Path):
    layer = saved(tmp_path, *CHAIN)
    remove_files(tmp_path, 'roads')
    ran = check(tmp_path, entry(DANGLES, layer))
    assert not ran.ok
    assert ran.log.strip()


def test_an_output_on_an_input_file_is_refused_and_the_input_kept(
    tmp_path: Path,
):
    layer = saved(tmp_path, *CHAIN)
    path = Path(layer.source().split('|')[0])
    before = path.read_bytes()
    ran = check(tmp_path, entry(DANGLES, layer), output=path)
    assert not ran.ok
    assert 'input' in ran.log.lower()
    assert path.read_bytes() == before


def test_an_output_on_the_second_input_file_is_refused(tmp_path: Path):
    lines = saved(tmp_path, *CHAIN)
    points = point_layer('POINT(5 0)')
    QgsProject.instance().addMapLayer(points)
    path = Path(lines.source().split('|')[0])
    before = path.read_bytes()
    ran = check(tmp_path, entry(COVERED, points, lines), output=path)
    assert not ran.ok
    assert path.read_bytes() == before


def test_an_output_without_the_extension_gets_one(tmp_path: Path):
    layer = saved(tmp_path, *CHAIN)
    ran = check(tmp_path, entry(DANGLES, layer), output=tmp_path / 'errors')
    assert ran.ok, ran.log
    assert (tmp_path / 'errors.gpkg').exists()


def test_a_layer_edited_but_not_saved_is_checked_as_edited(tmp_path: Path):
    layer = saved(tmp_path, *CHAIN)
    assert layer.startEditing()
    feature = QgsFeature(layer.fields())
    feature.setGeometry(QgsGeometry.fromWkt(FAR))
    assert layer.addFeature(feature)
    try:
        ran = check(tmp_path, entry(DANGLES, layer))
    finally:
        layer.rollBack()
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 4


def test_a_memory_layer_is_checked(tmp_path: Path):
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    ran = check(tmp_path, entry(DANGLES, layer))
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 2


@pytest.mark.parametrize(
    'rules',
    ['', 'not json', '[]', '{}', '[1]', '[{"rule": 1, "layer": "x"}]'],
)
def test_rules_that_are_not_a_list_of_rules(tmp_path: Path, rules: str):
    ran = run_algorithm(
        'check_rules', {'RULES': rules, 'OUTPUT': str(tmp_path / 'e.gpkg')}
    )
    assert not ran.ok
    assert ran.log.strip()


def test_a_rule_the_executable_does_not_have(tmp_path: Path):
    layer = saved(tmp_path, *CHAIN)
    ran = check(tmp_path, entry('line/must-not-exist', layer))
    assert not ran.ok
    assert 'Unknown rule' in ran.log


def test_a_two_layer_rule_without_the_second_layer(tmp_path: Path):
    layer = saved(tmp_path, *CHAIN)
    ran = check(tmp_path, entry(COVERED, layer))
    assert not ran.ok


def test_a_one_layer_rule_with_a_second_layer(tmp_path: Path):
    layer = saved(tmp_path, *CHAIN)
    ran = check(tmp_path, entry(DANGLES, layer, layer))
    assert not ran.ok


def errors_in(output: object, name: str) -> QgsVectorLayer:
    layer = QgsVectorLayer(f'{output}|layername={name}', name, 'ogr')
    assert layer.isValid()
    return layer


def test_error_ids_are_the_ids_of_a_layer_with_a_hole(tmp_path: Path):
    layer = line_layer(FAR, *CHAIN)
    QgsProject.instance().addMapLayer(layer)
    assert layer.dataProvider().deleteFeatures([1])
    ran = check(tmp_path, entry(DANGLES, layer))
    assert ran.ok, ran.log
    errors = errors_in(ran.results['OUTPUT'], 'errors_point')
    assert errors.featureCount() == 2
    for error in errors.getFeatures():
        assert error['origin_fid'] in (2, 3)
        feature = layer.getFeature(int(error['origin_fid']))
        assert feature.isValid()
        assert feature.geometry().intersects(error.geometry())


def test_the_ids_of_the_other_layer_are_the_source_ids(tmp_path: Path):
    first = line_layer('LINESTRING(0 0, 10 0)', name='first')
    second = line_layer(
        'LINESTRING(0 100, 1 100)', 'LINESTRING(5 0, 15 0)', name='second'
    )
    for layer in (first, second):
        QgsProject.instance().addMapLayer(layer)
    assert second.dataProvider().deleteFeatures([1])
    ran = check(tmp_path, entry('line/must-not-overlap-with', first, second))
    assert ran.ok, ran.log
    error = next(errors_in(ran.results['OUTPUT'], 'errors_line').getFeatures())
    assert (error['origin_fid'], error['destination_fid']) == (1, 2)
    assert second.getFeature(2).isValid()


def test_error_ids_of_a_layer_saved_with_a_hole(tmp_path: Path):
    layer = saved(tmp_path, FAR, *CHAIN)
    assert layer.startEditing()
    assert layer.deleteFeature(1)
    assert layer.commitChanges()
    ran = check(tmp_path, entry(DANGLES, layer))
    assert ran.ok, ran.log
    errors = errors_in(ran.results['OUTPUT'], 'errors_point')
    for error in errors.getFeatures():
        feature = layer.getFeature(int(error['origin_fid']))
        assert feature.geometry().intersects(error.geometry())
