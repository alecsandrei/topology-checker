"""Layers in formats that the tool does not read. The tool of a default build
reads GeoJSON, Shapefile, FlatGeobuf, GeoPackage, GeoParquet, KML and GPX. A
layer in any other format that QGIS reads is still checked, through an export,
and a file that the tool reads in many layers is checked for the one layer."""

from __future__ import annotations

import typing as t
from pathlib import Path

import pytest
import tomllib
from qgis.core import (
    QgsProject,
    QgsVectorFileWriter,
    QgsVectorLayer,
)

from topology_rules.panel.dock import Dock

from .helpers import add, cli_counts, click_run, line_layer, queue_column, shown

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
# The formats that the tool does not read: driver, suffix, and the options of
# the data source and of the layer when it is written
UNREAD = {
    'spatialite': ('SQLite', '.sqlite', ['SPATIALITE=YES'], []),
    'gml': ('GML', '.gml', [], []),
    'csv': ('CSV', '.csv', [], ['GEOMETRY=AS_WKT']),
}
# The suffixes of the files that the tool reads, and that a definition may
# name
READ_BY_THE_TOOL = {
    '.gpkg',
    '.shp',
    '.geojson',
    '.json',
    '.fgb',
    '.parquet',
    '.geoparquet',
    '.kml',
    '.kmz',
    '.gpx',
}


def written(
    path: Path,
    driver: str,
    source_options: list[str],
    layer_options: list[str],
    name: str,
    action: t.Any,
) -> None:
    layer = line_layer(*CHAIN, name=name)
    options = QgsVectorFileWriter.SaveVectorOptions()
    options.driverName = driver
    options.layerName = name
    options.datasourceOptions = source_options
    options.layerOptions = layer_options
    options.actionOnExistingFile = action
    result = QgsVectorFileWriter.writeAsVectorFormatV3(
        layer, str(path), QgsProject.instance().transformContext(), options
    )
    if result[0] != QgsVectorFileWriter.WriterError.NoError:
        pytest.skip(f'this QGIS cannot write {driver}: {result[1]}')


def in_format(kind: str, directory: Path) -> QgsVectorLayer:
    """A layer of two lines in the format, added to the project."""
    driver, suffix, source_options, layer_options = UNREAD[kind]
    path = directory / f'roads{suffix}'
    written(
        path,
        driver,
        source_options,
        layer_options,
        'roads',
        QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteFile,
    )
    uri = f'{path}|layername=roads' if kind == 'spatialite' else str(path)
    layer = QgsVectorLayer(uri, 'roads', 'ogr')
    if not layer.isValid():
        pytest.skip(f'this QGIS cannot read {driver}')
    QgsProject.instance().addMapLayer(layer)
    return layer


@pytest.mark.parametrize('kind', UNREAD)
def test_a_layer_in_a_format_the_tool_does_not_read_is_checked(
    dock: Dock, tmp_path: Path, kind: str
):
    add(dock, DANGLES, in_format(kind, tmp_path))
    click_run(dock)
    assert shown(dock) == 2


@pytest.mark.parametrize('kind', UNREAD)
def test_such_a_layer_is_exported_in_the_geopackage_next_to_the_file(
    dock: Dock, tmp_path: Path, kind: str
):
    (tmp_path / 'data').mkdir()
    add(dock, DANGLES, in_format(kind, tmp_path / 'data'))
    toml = tmp_path / 'topology.toml'
    dock.export_rules(toml)
    config = tomllib.loads(toml.read_text(encoding='utf-8'))
    paths = [Path(found['path']) for found in config['layers'].values()]
    assert paths
    assert all(path.suffix.lower() in READ_BY_THE_TOOL for path in paths)
    assert (tmp_path / 'topology_data.gpkg').is_file()


@pytest.mark.parametrize('kind', UNREAD)
def test_the_exported_file_of_such_a_layer_gives_the_counts_of_the_panel(
    dock: Dock, tmp_path: Path, executable: str, kind: str
):
    (tmp_path / 'data').mkdir()
    add(dock, DANGLES, in_format(kind, tmp_path / 'data'))
    toml = tmp_path / 'topology.toml'
    dock.export_rules(toml)
    exported = cli_counts(executable, toml)
    click_run(dock)
    assert exported == [int(count) for count in queue_column(dock, 3)]


# A file with many layers that the tool reads, such as KML


def kml_with_two_layers(directory: Path) -> QgsVectorLayer:
    """A KML of two layers, 'roads' and 'rails', each of two lines, and the
    layer 'roads' of it in the project."""
    path = directory / 'both.kml'
    for name, action in (
        (
            'roads',
            QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteFile,
        ),
        (
            'rails',
            QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteLayer,
        ),
    ):
        written(path, 'KML', [], [], name, action)
    layer = QgsVectorLayer(f'{path}|layername=roads', 'roads', 'ogr')
    if not layer.isValid():
        pytest.skip('this QGIS cannot read KML')
    if (
        QgsVectorLayer(f'{path}|layername=rails', 'rails', 'ogr').featureCount()
        != 2
    ):
        pytest.skip('this KML holds only one layer')
    QgsProject.instance().addMapLayer(layer)
    return layer


def test_a_layer_of_a_kml_is_checked_alone(dock: Dock, tmp_path: Path):
    add(dock, DANGLES, kml_with_two_layers(tmp_path))
    click_run(dock)
    assert shown(dock) == 2


def test_the_exported_file_of_a_layer_of_a_kml_checks_it_alone(
    dock: Dock, tmp_path: Path, executable: str
):
    (tmp_path / 'data').mkdir()
    add(dock, DANGLES, kml_with_two_layers(tmp_path / 'data'))
    toml = tmp_path / 'topology.toml'
    dock.export_rules(toml)
    assert cli_counts(executable, toml) == [2]
