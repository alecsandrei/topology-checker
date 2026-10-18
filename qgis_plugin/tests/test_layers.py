"""Preparing layers for the tool: its own file or an export of the features."""

from __future__ import annotations

from pathlib import Path

import pytest
from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsVectorLayer,
)

from topology_rules.core.inputs import prepare
from topology_rules.core.layers import Dataset, Input, file_dataset
from topology_rules.exceptions import RunError

from .helpers import files_stay_open, line_layer, remove_files, save

LINE = 'LINESTRING(0 0, 1 1)'


def geopackage_layer(tmp_path: Path) -> QgsVectorLayer:
    path = tmp_path / 'data.gpkg'
    save(line_layer(LINE), path, 'GPKG', 'roads')
    return QgsVectorLayer(f'{path}|layername=roads', 'roads', 'ogr')


def run_prepare(
    item: Input,
    directory: Path,
    target: str | None = None,
) -> tuple[object, str]:
    feedback = QgsProcessingFeedback()
    crs = None if target is None else QgsCoordinateReferenceSystem(target)
    prepared = prepare(item, directory, crs, QgsProcessingContext(), feedback)
    return prepared, feedback.textLog()


def test_geopackage_layer_is_passed_by_path_and_name(tmp_path: Path):
    layer = geopackage_layer(tmp_path)
    assert file_dataset(layer) == Dataset(str(tmp_path / 'data.gpkg'), 'roads')


@pytest.mark.parametrize(
    ('driver', 'name'),
    [('GeoJSON', 'roads.geojson'), ('ESRI Shapefile', 'roads.shp')],
)
def test_single_layer_files_are_exported(
    tmp_path: Path, driver: str, name: str
):
    # The tool numbers their features differently from QGIS
    path = tmp_path / name
    save(line_layer(LINE), path, driver, 'roads')
    assert file_dataset(QgsVectorLayer(str(path), 'roads', 'ogr')) is None


def test_filtered_layer_is_exported(tmp_path: Path):
    layer = geopackage_layer(tmp_path)
    layer.setSubsetString('"fid" > 100')
    assert file_dataset(layer) is None


def test_edited_layer_is_exported(tmp_path: Path):
    layer = geopackage_layer(tmp_path)
    assert layer.startEditing()
    assert layer.deleteFeature(1)
    try:
        assert file_dataset(layer) is None
    finally:
        layer.rollBack()


def test_geopackage_layer_is_prepared_as_it_is(tmp_path: Path):
    layer = geopackage_layer(tmp_path)
    prepared, log = run_prepare(Input('roads', layer, layer), tmp_path)
    assert prepared.dataset == Dataset(str(tmp_path / 'data.gpkg'), 'roads')
    assert prepared.ids is None
    assert log == ''


def test_memory_layer_is_exported_to_a_geopackage(tmp_path: Path):
    layer = line_layer(LINE, name='my lines')
    prepared, _ = run_prepare(Input('my lines', layer, layer), tmp_path)
    assert Path(prepared.dataset.path).parent == tmp_path
    assert Path(prepared.dataset.path).name == 'my_lines.gpkg'
    assert prepared.dataset.layer == 'my_lines'
    reread = QgsVectorLayer(
        f'{prepared.dataset.path}|layername=my_lines', 'x', 'ogr'
    )
    assert reread.isValid()
    assert reread.featureCount() == 1


def test_same_named_layers_do_not_overwrite_each_other(tmp_path: Path):
    first, _ = run_prepare(Input('lines', line_layer(LINE)), tmp_path)
    second, _ = run_prepare(Input('lines', line_layer(LINE)), tmp_path)
    assert first.dataset.path != second.dataset.path


def test_the_ids_map_the_exported_features_to_the_source_ids(tmp_path: Path):
    layer = line_layer(LINE, 'LINESTRING(5 5, 6 6)', 'LINESTRING(8 8, 9 9)')
    assert layer.dataProvider().deleteFeatures([2])
    prepared, _ = run_prepare(Input('lines', layer, layer), tmp_path)
    assert prepared.ids == (1, 3)
    reread = QgsVectorLayer(
        f'{prepared.dataset.path}|layername=lines', 'x', 'ogr'
    )
    assert reread.featureCount() == 2


def test_a_selection_is_exported_with_its_ids(tmp_path: Path):
    layer = geopackage_layer(tmp_path)
    prepared, _ = run_prepare(Input('roads', layer, layer, False), tmp_path)
    assert prepared.ids == (1,)
    assert prepared.dataset.path != str(tmp_path / 'data.gpkg')


def test_a_reprojection_is_logged(tmp_path: Path):
    layer = line_layer(LINE)
    prepared, log = run_prepare(
        Input('lines', layer, layer), tmp_path, 'EPSG:4326'
    )
    assert 'Reprojecting lines from EPSG:32633 to EPSG:4326' in log
    reread = QgsVectorLayer(
        f'{prepared.dataset.path}|layername=lines', 'x', 'ogr'
    )
    assert reread.crs().authid() == 'EPSG:4326'


def test_an_unchanged_crs_logs_nothing(tmp_path: Path):
    layer = line_layer(LINE)
    _, log = run_prepare(Input('lines', layer, layer), tmp_path, 'EPSG:32633')
    assert 'Reprojecting' not in log


def test_a_geopackage_layer_in_another_crs_is_exported(tmp_path: Path):
    layer = geopackage_layer(tmp_path)
    prepared, log = run_prepare(
        Input('roads', layer, layer), tmp_path, 'EPSG:4326'
    )
    assert prepared.ids == (1,)
    assert 'Reprojecting' in log


def test_an_invalid_layer_is_refused(tmp_path: Path):
    layer = QgsVectorLayer(str(tmp_path / 'nothing.gpkg'), 'gone', 'ogr')
    with pytest.raises(RunError, match='not valid'):
        run_prepare(Input('gone', layer, layer), tmp_path)


@files_stay_open
def test_a_layer_whose_file_is_gone_is_refused(tmp_path: Path):
    layer = geopackage_layer(tmp_path)
    remove_files(tmp_path, 'data')
    with pytest.raises(RunError, match='gone'):
        run_prepare(Input('roads', layer, layer), tmp_path)
