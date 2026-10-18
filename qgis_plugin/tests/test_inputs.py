"""The layers users have in a project, which are rarely one memory layer."""

from __future__ import annotations

import collections.abc as c
import zipfile
from pathlib import Path

import pytest
from qgis.core import (
    QgsFeature,
    QgsGeometry,
    QgsProject,
    QgsProviderRegistry,
    QgsVectorFileWriter,
    QgsVectorLayer,
)

from topology_rules.algorithms.rule import COUNT

from .helpers import (
    VANISHING,
    add_features,
    line_layer,
    remove_files,
    run_algorithm,
    save,
    saved_layer,
)

DANGLES = 'line_must_not_have_dangles'
# Two lines meeting at a node: the free end of each is a dangle
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
# A line far from the chain, with two more dangles
FAR = 'LINESTRING(0 50, 10 50)'


@pytest.fixture(autouse=True)
def ready(executable: str, provider: object) -> None:
    """The real executable and the provider, as in a QGIS session."""


def dangles(layer: QgsVectorLayer, tmp_path: Path, name: str = DANGLES):
    QgsProject.instance().addMapLayer(layer)
    return run_algorithm(name, {'INPUT': layer.id()})


def memory(tmp_path: Path) -> QgsVectorLayer:
    return line_layer(*CHAIN)


def geopackage(tmp_path: Path) -> QgsVectorLayer:
    return saved_layer(tmp_path, CHAIN)


def geopackage_in_a_folder_with_spaces(tmp_path: Path) -> QgsVectorLayer:
    folder = tmp_path / 'my data' / 'ünï cödé'
    folder.mkdir(parents=True)
    return saved_layer(folder, CHAIN, name='stræt lines')


def geopackage_by_index(tmp_path: Path) -> QgsVectorLayer:
    layer = saved_layer(tmp_path, CHAIN)
    path = layer.source().split('|')[0]
    return QgsVectorLayer(f'{path}|layerid=0', 'by index', 'ogr')


def geopackage_with_other_layers(tmp_path: Path) -> QgsVectorLayer:
    path = tmp_path / 'lines.gpkg'
    # A second layer in the same file as the layer to check
    save(line_layer(*CHAIN), path, 'GPKG', 'chain')
    options = QgsVectorFileWriter.SaveVectorOptions()
    options.driverName = 'GPKG'
    options.layerName = 'far'
    options.actionOnExistingFile = (
        QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteLayer
    )
    QgsVectorFileWriter.writeAsVectorFormatV3(
        line_layer(FAR),
        str(path),
        QgsProject.instance().transformContext(),
        options,
    )
    return QgsVectorLayer(f'{path}|layername=chain', 'chain', 'ogr')


def shapefile(tmp_path: Path) -> QgsVectorLayer:
    return saved_layer(tmp_path, CHAIN, driver='ESRI Shapefile')


def geojson(tmp_path: Path) -> QgsVectorLayer:
    return saved_layer(tmp_path, CHAIN, driver='GeoJSON')


def zipped_shapefile(tmp_path: Path) -> QgsVectorLayer:
    folder = tmp_path / 'shape'
    folder.mkdir()
    saved_layer(folder, CHAIN, driver='ESRI Shapefile')
    archive = tmp_path / 'lines.zip'
    with zipfile.ZipFile(archive, 'w') as zipped:
        for part in folder.iterdir():
            zipped.write(part, part.name)
    return QgsVectorLayer(f'/vsizip/{archive}/lines.shp', 'zipped', 'ogr')


def delimited_text(tmp_path: Path) -> QgsVectorLayer:
    registry = QgsProviderRegistry.instance()
    if registry.providerMetadata('delimitedtext') is None:
        pytest.skip('this QGIS has no delimited text provider')
    path = tmp_path / 'lines.csv'
    path.write_text('wkt\n' + '\n'.join(f'"{wkt}"' for wkt in CHAIN) + '\n')
    uri = f'{path.as_uri()}?delimiter=,&wktField=wkt&crs=EPSG:32633'
    return QgsVectorLayer(uri, 'csv', 'delimitedtext')


ORIGINS = {
    'memory': memory,
    'geopackage': geopackage,
    'path with spaces and accents': geopackage_in_a_folder_with_spaces,
    'geopackage layer by index': geopackage_by_index,
    'geopackage with other layers': geopackage_with_other_layers,
    'shapefile': shapefile,
    'geojson': geojson,
    'zipped shapefile': zipped_shapefile,
    'delimited text': delimited_text,
}


# Every origin in a task is checked by test_sources, with the ids of the
# errors; here is the same run without a task, as the Python console does it
@pytest.mark.parametrize('origin', ['memory', 'geopackage'])
def test_layer_origins_without_a_task(origin: str, tmp_path: Path):
    layer = ORIGINS[origin](tmp_path)
    assert layer.isValid()
    QgsProject.instance().addMapLayer(layer)
    ran = run_algorithm(DANGLES, {'INPUT': layer.id()}, threaded=False)
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 2


def test_a_filtered_layer_is_checked_as_filtered(tmp_path: Path):
    layer = saved_layer(tmp_path, (*CHAIN, FAR))
    layer.setSubsetString('fid <= 2')
    ran = dangles(layer, tmp_path)
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 2


def test_edits_that_are_not_saved_are_checked(tmp_path: Path):
    layer = saved_layer(tmp_path, CHAIN)
    QgsProject.instance().addMapLayer(layer)
    assert layer.startEditing()
    feature = QgsFeature(layer.fields())
    feature.setGeometry(QgsGeometry.fromWkt(FAR))
    assert layer.addFeature(feature)
    ran = run_algorithm(
        DANGLES,
        {'INPUT': layer.id()},
    )
    layer.rollBack()
    assert ran.ok, ran.log
    # The chain and the line that only the edit buffer has
    assert ran.results[COUNT] == 4


def test_edits_of_a_memory_layer_during_a_run(tmp_path: Path):
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    assert layer.startEditing()
    feature = QgsFeature(layer.fields())
    feature.setGeometry(QgsGeometry.fromWkt(FAR))
    assert layer.addFeature(feature)
    ran = run_algorithm(
        DANGLES,
        {'INPUT': layer.id()},
    )
    layer.rollBack()
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 4


def test_selected_features_do_not_limit_the_check(tmp_path: Path):
    layer = saved_layer(tmp_path, (*CHAIN, FAR))
    QgsProject.instance().addMapLayer(layer)
    layer.selectByIds([1])
    ran = run_algorithm(
        DANGLES,
        {'INPUT': layer.id()},
    )
    assert ran.ok, ran.log
    # The whole layer, as the tool does when it reads the file
    assert ran.results[COUNT] == 4


def test_a_layer_checked_many_times_in_a_row(tmp_path: Path):
    layer = saved_layer(tmp_path, CHAIN)
    QgsProject.instance().addMapLayer(layer)
    for _ in range(5):
        ran = run_algorithm(
            DANGLES,
            {'INPUT': layer.id()},
        )
        assert ran.ok, ran.log
        assert ran.results[COUNT] == 2


def test_a_big_memory_layer_in_a_task(tmp_path: Path):
    far = [f'LINESTRING(0 {y}, 5 {y})' for y in range(1, 101)]
    layer = line_layer(*CHAIN, *far)
    QgsProject.instance().addMapLayer(layer)
    ran = run_algorithm(
        DANGLES,
        {'INPUT': layer.id()},
    )
    assert ran.ok, ran.log
    assert ran.results[COUNT] == 202


def odd(kind: str, crs: str, *wkts: str) -> c.Callable[[], QgsVectorLayer]:
    """A memory layer, made when the test runs and QGIS exists."""

    def make() -> QgsVectorLayer:
        layer = QgsVectorLayer(
            f'{kind}?crs={crs}' if crs else kind, 'odd', 'memory'
        )
        add_features(layer, wkts)
        return layer

    return make


LINE = 'LineString'
UTM = 'EPSG:32633'


@pytest.mark.parametrize(
    ('make', 'count'),
    [
        pytest.param(odd(LINE, 'EPSG:4326', *CHAIN), 2, id='4326'),
        pytest.param(odd(LINE, 'EPSG:2154', *CHAIN), 2, id='2154'),
        pytest.param(odd(LINE, UTM), 0, id='empty'),
        pytest.param(
            odd(LINE, UTM, *CHAIN, 'LINESTRING EMPTY'), 2, id='empty geometry'
        ),
        pytest.param(
            odd(
                'LineStringZ',
                UTM,
                'LINESTRING Z (0 0 5, 10 0 5)',
                'LINESTRING Z (10 0 5, 20 0 5)',
            ),
            2,
            id='z',
        ),
        pytest.param(
            odd(
                'MultiLineString',
                UTM,
                'MULTILINESTRING((0 0, 10 0), (10 0, 20 0))',
            ),
            2,
            id='multi',
        ),
    ],
)
def test_odd_but_valid_data(
    make: c.Callable[[], QgsVectorLayer], count: int, tmp_path: Path
):
    ran = dangles(make(), tmp_path)
    assert ran.ok, ran.log
    assert ran.results[COUNT] == count


@pytest.mark.parametrize(
    'make',
    [
        pytest.param(odd(LINE, '', *CHAIN), id='no crs'),
        pytest.param(
            odd(LINE, UTM, 'LINESTRING(0 0, 0 0)', *CHAIN), id='zero length'
        ),
        pytest.param(
            odd(LINE, UTM, 'LINESTRING(0 0, 10 10, 10 0, 0 10)'),
            id='self crossing',
        ),
        pytest.param(
            odd(
                'CompoundCurve',
                UTM,
                'COMPOUNDCURVE(CIRCULARSTRING(0 0, 1 1, 2 0))',
            ),
            id='curve',
        ),
    ],
)
def test_data_that_may_not_be_accepted_says_why(
    make: c.Callable[[], QgsVectorLayer], tmp_path: Path
):
    ran = dangles(make(), tmp_path)
    assert ran.ok or ran.log.strip()


@pytest.mark.parametrize(
    ('name', 'make'),
    [
        ('line_must_not_have_dangles', odd('Point', UTM, 'POINT(0 0)')),
        ('point_must_not_overlap', odd(LINE, UTM, *CHAIN)),
        ('polygon_must_not_overlap', odd(LINE, UTM, *CHAIN)),
    ],
)
def test_a_layer_of_the_wrong_kind_is_refused(
    name: str, make: c.Callable[[], QgsVectorLayer], tmp_path: Path
):
    ran = dangles(make(), tmp_path, name)
    assert not ran.ok
    assert ran.log.strip()


def test_a_table_without_geometry_is_refused(tmp_path: Path):
    layer = QgsVectorLayer('None', 'table', 'memory')
    ran = dangles(layer, tmp_path)
    assert not ran.ok
    assert ran.log.strip()


def test_a_layer_whose_file_vanished_after_it_was_added(tmp_path: Path):
    layer = saved_layer(tmp_path, CHAIN, driver=VANISHING)
    QgsProject.instance().addMapLayer(layer)
    remove_files(tmp_path, 'lines')
    ran = run_algorithm(
        DANGLES,
        {'INPUT': layer.id()},
    )
    assert not ran.ok
    assert ran.log.strip()


def test_utilities_on_a_saved_layer(tmp_path: Path):
    layer = saved_layer(tmp_path, CHAIN)
    QgsProject.instance().addMapLayer(layer)
    ran = run_algorithm(
        'merge_lines', {'INPUT': layer.id(), 'OUTPUT': 'memory:output'}
    )
    assert ran.ok, ran.log
    output = ran.context.getMapLayer(str(ran.results['OUTPUT']))
    assert output is not None
    assert output.featureCount() == 1
