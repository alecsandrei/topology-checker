"""The list of rules as a topology.toml, which the command line also runs."""

from __future__ import annotations

import tomllib
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsFeature, QgsGeometry, QgsProject, QgsVectorLayer
from qgis.PyQt.QtWidgets import QFileDialog

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    checked,
    cli_counts,
    click_run,
    line_layer,
    message_text,
    queue_column,
    saved_layer,
    saved_points,
    set_checked,
    write_toml,
)
from .test_inputs import geopackage_with_other_layers

DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
SQUARE_RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)


def add_to_project(layer: QgsVectorLayer) -> QgsVectorLayer:
    QgsProject.instance().addMapLayer(layer)
    return layer


@pytest.fixture
def data(tmp_path: Path) -> Path:
    folder = tmp_path / 'data'
    folder.mkdir()
    return folder


@pytest.fixture
def filled(dock: Dock, data: Path) -> Dock:
    """Dangles on roads, points that must lie on the roads, and a rule that is
    switched off."""
    roads = add_to_project(saved_layer(data, CHAIN, 'roads'))
    rails = add_to_project(saved_layer(data, CHAIN, 'rails'))
    stops = add_to_project(saved_points(data, ['POINT(5 0)', 'POINT(5 5)']))
    add(dock, DANGLES, roads)
    add(dock, COVERED, stops, roads)
    add(dock, DANGLES, rails)
    set_checked(dock, 2, False)
    return dock


def read(path: Path) -> dict[str, t.Any]:
    return tomllib.loads(path.read_text(encoding='utf-8'))


def test_there_is_an_export_button(dock: Dock):
    assert dock.export_button.text() == 'Export…'


def test_the_checked_rows_are_exported_as_rules(filled: Dock, tmp_path: Path):
    path = tmp_path / 'topology.toml'
    filled.export_rules(path)
    rules = read(path)['rules']
    assert [rule['rule'] for rule in rules] == [DANGLES, COVERED]


def test_the_rules_name_their_layers_by_alias(filled: Dock, tmp_path: Path):
    path = tmp_path / 'topology.toml'
    filled.export_rules(path)
    config = read(path)
    first, second = config['rules']
    assert first['layer'] == 'roads'
    assert second['layer'] == 'stops'
    assert second['other'] == 'roads'
    assert set(config['layers']) == {'roads', 'stops'}


def test_the_layers_of_geopackages_are_named(filled: Dock, tmp_path: Path):
    path = tmp_path / 'topology.toml'
    filled.export_rules(path)
    layers = read(path)['layers']
    assert layers['roads']['layer'] == 'roads'
    assert layers['stops']['layer'] == 'stops'


def test_the_paths_are_relative_to_the_folder_of_the_file(
    filled: Dock, tmp_path: Path
):
    path = tmp_path / 'topology.toml'
    filled.export_rules(path)
    layers = read(path)['layers']
    assert Path(layers['roads']['path']).as_posix() == 'data/roads.gpkg'


def test_the_paths_are_relative_to_a_file_elsewhere(
    filled: Dock, tmp_path: Path
):
    folder = tmp_path / 'rules' / 'deep'
    folder.mkdir(parents=True)
    path = folder / 'topology.toml'
    filled.export_rules(path)
    layers = read(path)['layers']
    assert Path(layers['roads']['path']).as_posix() == '../../data/roads.gpkg'


def test_layers_with_the_same_name_get_unique_aliases(
    dock: Dock, data: Path, tmp_path: Path
):
    first = data / 'a'
    second = data / 'b'
    first.mkdir()
    second.mkdir()
    add(dock, DANGLES, add_to_project(saved_layer(first, CHAIN, 'roads')))
    add(dock, DANGLES, add_to_project(saved_layer(second, CHAIN, 'roads')))
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    config = read(path)
    assert len(config['layers']) == 2
    assert all(alias.startswith('roads') for alias in config['layers'])
    assert config['rules'][0]['layer'] != config['rules'][1]['layer']


def test_the_tolerance_is_exported_when_it_is_set(filled: Dock, tmp_path: Path):
    filled.tolerance.setValue(0.5)
    path = tmp_path / 'topology.toml'
    filled.export_rules(path)
    assert read(path)['tolerance'] == 0.5


def test_the_default_tolerance_is_not_exported(filled: Dock, tmp_path: Path):
    path = tmp_path / 'topology.toml'
    filled.export_rules(path)
    assert 'tolerance' not in read(path)


def test_the_exported_file_gives_the_counts_of_the_dock(
    filled: Dock, tmp_path: Path, executable: str
):
    click_run(filled)
    path = tmp_path / 'topology.toml'
    filled.export_rules(path)
    expected = [int(text) for text in queue_column(filled, 3) if text]
    assert cli_counts(executable, path) == expected == [2, 1]


def test_the_exported_file_with_a_tolerance_gives_the_counts_of_the_dock(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    gap = saved_layer(
        data, ('LINESTRING(0 0, 10 0)', 'LINESTRING(10.4 0, 20 0)'), 'gap'
    )
    add(dock, DANGLES, add_to_project(gap))
    dock.tolerance.setValue(1.0)
    click_run(dock)
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    assert cli_counts(executable, path) == [2]
    assert queue_column(dock, 3) == ['2']


def memory_layer(data: Path) -> QgsVectorLayer:
    return add_to_project(line_layer(*CHAIN, name='scratch'))


def filtered_layer(data: Path) -> QgsVectorLayer:
    layer = add_to_project(saved_layer(data, CHAIN, 'filtered'))
    layer.setSubsetString('"fid" > 0')
    return layer


def edited_layer(data: Path) -> QgsVectorLayer:
    layer = add_to_project(saved_layer(data, CHAIN, 'edited'))
    assert layer.startEditing()
    feature = QgsFeature(layer.fields())
    feature.setGeometry(QgsGeometry.fromWkt('LINESTRING(0 9, 1 9)'))
    assert layer.addFeature(feature)
    return layer


KINDS = {
    'scratch': memory_layer,
    'filtered': filtered_layer,
    'edited': edited_layer,
}


def test_a_rule_that_is_switched_off_does_not_stop_the_export(
    dock: Dock, data: Path, tmp_path: Path
):
    add(dock, DANGLES, add_to_project(saved_layer(data, CHAIN, 'roads')))
    add(dock, DANGLES, memory_layer(data))
    set_checked(dock, 1, False)
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    assert [rule['layer'] for rule in read(path)['rules']] == ['roads']


def test_exporting_nothing_writes_nothing(
    dock: Dock, tmp_path: Path, bar: t.Any
):
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    assert not path.exists()
    assert message_text(bar)


def test_the_export_button_asks_for_a_file(
    filled: Dock, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    path = tmp_path / 'chosen.toml'
    monkeypatch.setattr(
        QFileDialog, 'getSaveFileName', lambda *args, **kwargs: (str(path), '')
    )
    filled.export_button.click()
    assert path.exists()


# Import


def definition(data: Path, tmp_path: Path, **options: t.Any) -> Path:
    """A topology.toml next to the data folder, as a user keeps them."""
    saved_layer(data, CHAIN, 'roads')
    saved_points(data, ['POINT(5 0)', 'POINT(5 5)'])
    return write_toml(
        tmp_path / 'topology.toml',
        {
            'roads': ('data/roads.gpkg', 'roads'),
            'stops': ('data/stops.gpkg', 'stops'),
        },
        [(DANGLES, 'roads', None), (COVERED, 'stops', 'roads')],
        **options,
    )


def project_names() -> set[str]:
    return {
        layer.name() for layer in QgsProject.instance().mapLayers().values()
    }


def test_there_is_an_import_button(dock: Dock):
    assert dock.import_button.text() == 'Import…'


def test_importing_adds_the_rules(dock: Dock, data: Path, tmp_path: Path):
    dock.import_rules(definition(data, tmp_path))
    assert queue_column(dock, 0) == [
        'Must not have dangles',
        'Must be covered by lines',
    ]
    assert queue_column(dock, 1) == ['roads', 'stops']
    assert queue_column(dock, 2) == ['', 'roads']


def test_importing_adds_the_layers_that_the_project_lacks(
    dock: Dock, data: Path, tmp_path: Path
):
    dock.import_rules(definition(data, tmp_path))
    assert project_names() == {'roads', 'stops'}


def test_importing_reuses_a_layer_of_the_project(
    dock: Dock, data: Path, tmp_path: Path
):
    file = definition(data, tmp_path)
    existing = add_to_project(
        QgsVectorLayer(f'{data / "roads.gpkg"}|layername=roads', 'mine', 'ogr')
    )
    dock.import_rules(file)
    layers = QgsProject.instance().mapLayers()
    assert len(layers) == 2
    assert dock.entries[0].layer == existing.id()
    assert dock.entries[1].other == existing.id()


def test_a_layer_is_matched_by_file_and_layer_name(dock: Dock, tmp_path: Path):
    # A layer of the project from the same file, but another layer of it
    chain = add_to_project(geopackage_with_other_layers(tmp_path))
    file = write_toml(
        tmp_path / 'topology.toml',
        {'far': ('lines.gpkg', 'far')},
        [(DANGLES, 'far', None)],
    )
    dock.import_rules(file)
    assert dock.entries[0].layer != chain.id()
    assert project_names() == {'chain', 'far'}


def test_relative_paths_are_resolved_against_the_file(
    dock: Dock, data: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    file = definition(data, tmp_path)
    elsewhere = tmp_path / 'elsewhere'
    elsewhere.mkdir()
    monkeypatch.chdir(elsewhere)
    dock.import_rules(file)
    assert len(dock.entries) == 2


def test_importing_twice_skips_the_duplicates(
    dock: Dock, data: Path, tmp_path: Path
):
    file = definition(data, tmp_path)
    dock.import_rules(file)
    dock.import_rules(file)
    assert len(dock.entries) == 2
    assert len(QgsProject.instance().mapLayers()) == 2


def test_importing_appends_to_the_rules_of_the_list(
    dock: Dock, data: Path, tmp_path: Path
):
    mine = add_to_project(saved_layer(tmp_path, CHAIN, 'mine'))
    add(dock, DANGLES, mine)
    dock.import_rules(definition(data, tmp_path))
    assert queue_column(dock, 1) == ['mine', 'roads', 'stops']


def test_imported_rules_are_checked(dock: Dock, data: Path, tmp_path: Path):
    dock.import_rules(definition(data, tmp_path))
    assert checked(dock) == [True, True]


def test_the_tolerance_of_the_file_is_set(
    dock: Dock, data: Path, tmp_path: Path
):
    dock.import_rules(definition(data, tmp_path, tolerance=0.25))
    assert dock.tolerance.value() == 0.25


def test_a_file_without_a_tolerance_leaves_the_tolerance(
    dock: Dock, data: Path, tmp_path: Path
):
    dock.tolerance.setValue(0.75)
    dock.import_rules(definition(data, tmp_path))
    assert dock.tolerance.value() == 0.75


def test_an_imported_list_runs_like_the_file(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    file = definition(data, tmp_path)
    dock.import_rules(file)
    click_run(dock)
    expected = cli_counts(executable, file)
    assert [int(text) for text in queue_column(dock, 3)] == expected


def broken(tmp_path: Path, kind: str) -> Path:
    path = tmp_path / 'broken.toml'
    layers = {'roads': ('data/roads.gpkg', 'roads')}
    rules = [(DANGLES, 'roads', None)]
    if kind == 'invalid toml':
        path.write_text('this is = not [ toml', encoding='utf-8')
    elif kind == 'unknown rule':
        write_toml(path, layers, [('line/must-not-exist', 'roads', None)])
    elif kind == 'unknown key':
        write_toml(path, layers, rules)
        path.write_text('bogus = 1\n' + path.read_text(encoding='utf-8'))
    elif kind == 'unknown layer':
        write_toml(path, layers, [(DANGLES, 'nothing', None)])
    elif kind == 'missing data':
        write_toml(
            path,
            {'roads': ('data/gone.gpkg', 'roads')},
            rules,
        )
    elif kind == 'wrong rule layer count':
        write_toml(path, layers, [(COVERED, 'roads', None)])
    elif kind.endswith('tolerance'):
        write_toml(path, layers, rules)
        value = {
            'boolean tolerance': 'true',
            'negative tolerance': '-1',
            'text tolerance': '"wide"',
        }[kind]
        path.write_text(
            f'tolerance = {value}\n' + path.read_text(encoding='utf-8'),
            encoding='utf-8',
        )
    return path


@pytest.mark.parametrize(
    'kind',
    [
        'invalid toml',
        'unknown rule',
        'unknown key',
        'unknown layer',
        'missing data',
        'wrong rule layer count',
        'boolean tolerance',
        'negative tolerance',
        'text tolerance',
    ],
)
def test_a_file_that_cannot_be_used_leaves_the_list_unchanged(
    dock: Dock, data: Path, tmp_path: Path, bar: t.Any, kind: str
):
    saved_layer(data, CHAIN, 'roads')
    mine = add_to_project(saved_layer(tmp_path, CHAIN, 'mine'))
    add(dock, DANGLES, mine)
    dock.tolerance.setValue(0.75)
    dock.import_rules(broken(tmp_path, kind))
    assert queue_column(dock, 1) == ['mine']
    assert project_names() == {'mine'}
    assert dock.tolerance.value() == 0.75
    assert message_text(bar)


def test_a_file_that_does_not_exist_leaves_the_list_unchanged(
    dock: Dock, tmp_path: Path, bar: t.Any
):
    dock.import_rules(tmp_path / 'nothing.toml')
    assert dock.entries == []
    assert message_text(bar)


def test_the_import_button_asks_for_a_file(
    dock: Dock,
    data: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
):
    file = definition(data, tmp_path)
    monkeypatch.setattr(
        QFileDialog, 'getOpenFileName', lambda *args, **kwargs: (str(file), '')
    )
    dock.import_button.click()
    assert len(dock.entries) == 2
