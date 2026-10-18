"""Exporting rules on layers that are not files as they are: a filtered one,
one with unsaved edits, a temporary one. They are written into a GeoPackage
next to the TOML, as they are in the project."""

from __future__ import annotations

import os
import typing as t
from pathlib import Path

import pytest
import tomllib
from qgis.core import Qgis, QgsFeature, QgsGeometry, QgsProject, QgsVectorLayer
from qgis.PyQt.QtWidgets import QMessageBox

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    cli_counts,
    click_run,
    line_layer,
    message_text,
    queue_column,
    saved_layer,
)
from .test_definition_files import (
    CHAIN,
    DANGLES,
    add_to_project,
    edited_layer,
    filtered_layer,
    memory_layer,
)


def narrowed(data: Path) -> QgsVectorLayer:
    """A layer of two lines that the filter reduces to the second."""
    layer = filtered_layer(data)
    assert layer.setSubsetString('"fid" = 2')
    return layer


REASONS = {
    'scratch': 'temporary layer',
    'filtered': 'filter',
    'edited': 'unsaved edits',
}
MAKE = {
    'scratch': memory_layer,
    'filtered': narrowed,
    'edited': edited_layer,
}
# The name of a layer of the census, with a dash and accents
CENSUS = 'POP107D — Localități'


@pytest.fixture
def data(tmp_path: Path) -> Path:
    folder = tmp_path / 'data'
    folder.mkdir()
    return folder


@pytest.fixture
def asked(monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """The questions that the export asks, which are answered with Yes."""
    questions: list[str] = []

    def question(parent: object, title: str, text: str, *args: object) -> int:
        questions.append(text)
        return QMessageBox.StandardButton.Yes

    monkeypatch.setattr(QMessageBox, 'question', question)
    return questions


@pytest.fixture
def refused(monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """The questions that the export asks, which are answered with No."""
    questions: list[str] = []

    def question(parent: object, title: str, text: str, *args: object) -> int:
        questions.append(text)
        return QMessageBox.StandardButton.No

    monkeypatch.setattr(QMessageBox, 'question', question)
    return questions


def read(path: Path) -> dict[str, t.Any]:
    return tomllib.loads(path.read_text(encoding='utf-8'))


def stored(path: Path, name: str) -> QgsVectorLayer:
    layer = QgsVectorLayer(f'{path}|layername={name}', name, 'ogr')
    assert layer.isValid(), f'{name} is not in {path}'
    return layer


def info(bar: t.Any) -> list[str]:
    return list(bar.get_messages(Qgis.MessageLevel.Info))


def problems(bar: t.Any) -> list[str]:
    return [
        *bar.get_messages(Qgis.MessageLevel.Warning),
        *bar.get_messages(Qgis.MessageLevel.Critical),
    ]


def with_roads(dock: Dock, data: Path, kind: str) -> QgsVectorLayer:
    """Dangles on a layer of a file and on a layer of the given kind."""
    add(dock, DANGLES, add_to_project(saved_layer(data, CHAIN, 'roads')))
    layer = MAKE[kind](data)
    add(dock, DANGLES, layer)
    return layer


def rollback(layer: QgsVectorLayer) -> None:
    if layer.isEditable():
        layer.rollBack()


# The file next to the TOML


@pytest.mark.parametrize('kind', MAKE)
def test_the_layer_is_written_next_to_the_toml(
    dock: Dock, data: Path, tmp_path: Path, kind: str
):
    layer = with_roads(dock, data, kind)
    path = tmp_path / 'rules' / 'topology.toml'
    path.parent.mkdir()
    dock.export_rules(path)
    assert (path.parent / 'topology_data.gpkg').is_file()
    rollback(layer)


@pytest.mark.parametrize('kind', MAKE)
def test_the_toml_points_at_the_written_file(
    dock: Dock, data: Path, tmp_path: Path, kind: str
):
    layer = with_roads(dock, data, kind)
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    entry = read(path)['layers'][kind]
    assert Path(entry['path']).as_posix() == 'topology_data.gpkg'
    assert entry['layer'] == kind
    rollback(layer)


@pytest.mark.parametrize('kind', MAKE)
def test_a_layer_of_a_file_keeps_its_own_file(
    dock: Dock, data: Path, tmp_path: Path, kind: str
):
    layer = with_roads(dock, data, kind)
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    entry = read(path)['layers']['roads']
    assert Path(entry['path']).as_posix() == 'data/roads.gpkg'
    rollback(layer)


def test_the_file_is_named_after_the_toml(
    dock: Dock, data: Path, tmp_path: Path
):
    with_roads(dock, data, 'scratch')
    path = tmp_path / 'my rules.toml'
    dock.export_rules(path)
    assert (tmp_path / 'my rules_data.gpkg').is_file()
    assert not (tmp_path / 'topology_data.gpkg').exists()


def test_layers_of_files_alone_write_no_extra_file(
    dock: Dock, data: Path, tmp_path: Path, bar: t.Any
):
    add(dock, DANGLES, add_to_project(saved_layer(data, CHAIN, 'roads')))
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    assert path.is_file()
    assert not (tmp_path / 'topology_data.gpkg').exists()
    assert info(bar) == []


# What is written is what the project shows


def test_a_filter_is_kept(dock: Dock, data: Path, tmp_path: Path):
    layer = with_roads(dock, data, 'filtered')
    assert layer.featureCount() == 1
    dock.export_rules(tmp_path / 'topology.toml')
    assert (
        stored(tmp_path / 'topology_data.gpkg', 'filtered').featureCount() == 1
    )


def test_unsaved_edits_are_kept(dock: Dock, data: Path, tmp_path: Path):
    layer = with_roads(dock, data, 'edited')
    dock.export_rules(tmp_path / 'topology.toml')
    assert stored(tmp_path / 'topology_data.gpkg', 'edited').featureCount() == 3
    rollback(layer)


def test_a_temporary_layer_is_kept(dock: Dock, data: Path, tmp_path: Path):
    with_roads(dock, data, 'scratch')
    dock.export_rules(tmp_path / 'topology.toml')
    assert (
        stored(tmp_path / 'topology_data.gpkg', 'scratch').featureCount() == 2
    )


def test_the_geometry_and_the_crs_are_kept(
    dock: Dock, data: Path, tmp_path: Path
):
    with_roads(dock, data, 'scratch')
    dock.export_rules(tmp_path / 'topology.toml')
    written = stored(tmp_path / 'topology_data.gpkg', 'scratch')
    assert written.crs().authid() == 'EPSG:32633'
    assert sorted(f.geometry().asWkt() for f in written.getFeatures()) == [
        'LineString (0 0, 10 0)',
        'LineString (10 0, 20 0)',
    ]


def test_exporting_leaves_the_edits_in_the_layer(
    dock: Dock, data: Path, tmp_path: Path
):
    layer = with_roads(dock, data, 'edited')
    dock.export_rules(tmp_path / 'topology.toml')
    assert layer.isEditable()
    assert layer.isModified()
    assert layer.featureCount() == 3
    layer.rollBack()
    assert layer.featureCount() == 2


def test_exporting_leaves_the_filter_on_the_layer(
    dock: Dock, data: Path, tmp_path: Path
):
    layer = with_roads(dock, data, 'filtered')
    dock.export_rules(tmp_path / 'topology.toml')
    assert layer.subsetString() == '"fid" = 2'
    assert layer.featureCount() == 1


def test_the_attributes_do_not_matter(dock: Dock, data: Path, tmp_path: Path):
    layer = add_to_project(line_layer(*CHAIN, name='scratch'))
    assert layer.startEditing()
    feature = QgsFeature(layer.fields())
    feature.setGeometry(QgsGeometry.fromWkt('LINESTRING(0 5, 1 5)'))
    assert layer.addFeature(feature)
    assert layer.commitChanges()
    add(dock, DANGLES, layer)
    dock.export_rules(tmp_path / 'topology.toml')
    assert (
        stored(tmp_path / 'topology_data.gpkg', 'scratch').featureCount() == 3
    )


# The tool gives the counts of the dock


@pytest.mark.parametrize('kind', MAKE)
def test_the_exported_file_gives_the_counts_of_the_dock(
    dock: Dock, data: Path, tmp_path: Path, executable: str, kind: str
):
    layer = with_roads(dock, data, kind)
    click_run(dock)
    expected = [int(text) for text in queue_column(dock, 3) if text]
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    assert cli_counts(executable, path) == expected
    assert len(expected) == 2
    rollback(layer)


def test_all_kinds_together_give_the_counts_of_the_dock(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    add(dock, DANGLES, add_to_project(saved_layer(data, CHAIN, 'roads')))
    layers = [MAKE[kind](data) for kind in MAKE]
    for layer in layers:
        add(dock, DANGLES, layer)
    click_run(dock)
    expected = [int(text) for text in queue_column(dock, 3) if text]
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    assert cli_counts(executable, path) == expected
    assert len(expected) == 4
    for layer in layers:
        rollback(layer)


def test_a_rule_on_two_layers_that_are_not_files(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    first = add_to_project(line_layer(*CHAIN, name='first'))
    second = add_to_project(
        line_layer(
            'LINESTRING(5 0, 15 0)', 'LINESTRING(30 0, 40 0)', name='second'
        )
    )
    add(dock, 'line/must-not-overlap-with', first, second)
    click_run(dock)
    expected = [int(text) for text in queue_column(dock, 3) if text]
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    config = read(path)
    assert config['rules'][0]['layer'] == 'first'
    assert config['rules'][0]['other'] == 'second'
    assert cli_counts(executable, path) == expected


def test_the_tolerance_is_exported_with_the_data(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    layer = add_to_project(
        line_layer(
            'LINESTRING(0 0, 10 0)', 'LINESTRING(10.4 0, 20 0)', name='gap'
        )
    )
    add(dock, DANGLES, layer)
    dock.tolerance.setValue(1.0)
    click_run(dock)
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    assert read(path)['tolerance'] == 1.0
    assert cli_counts(executable, path) == [2]
    assert queue_column(dock, 3) == ['2']


# Names


def test_a_name_with_a_dash_and_accents(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    layer = add_to_project(line_layer(*CHAIN, name=CENSUS))
    add(dock, DANGLES, layer)
    click_run(dock)
    expected = [int(text) for text in queue_column(dock, 3) if text]
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    config = read(path)
    assert config['layers'][CENSUS]['layer'] == CENSUS
    assert config['rules'][0]['layer'] == CENSUS
    assert stored(tmp_path / 'topology_data.gpkg', CENSUS).featureCount() == 2
    assert cli_counts(executable, path) == expected


def test_a_toml_with_accents_and_spaces_in_its_name(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    layer = add_to_project(line_layer(*CHAIN, name=CENSUS))
    add(dock, DANGLES, layer)
    folder = tmp_path / 'Données — règles'
    folder.mkdir()
    path = folder / 'règles — ü.toml'
    dock.export_rules(path)
    assert (folder / 'règles — ü_data.gpkg').is_file()
    assert cli_counts(executable, path) == [2]


def test_two_layers_with_the_same_name_stay_apart(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    first = add_to_project(line_layer(*CHAIN, name='scratch'))
    second = add_to_project(line_layer('LINESTRING(0 0, 1 0)', name='scratch'))
    add(dock, DANGLES, first)
    add(dock, DANGLES, second)
    click_run(dock)
    expected = [int(text) for text in queue_column(dock, 3) if text]
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    layers = read(path)['layers']
    assert len(layers) == 2
    assert len({(v['path'], v['layer']) for v in layers.values()}) == 2
    assert cli_counts(executable, path) == expected == [2, 2]


def test_a_layer_used_by_two_rules_is_written_once(
    dock: Dock, data: Path, tmp_path: Path
):
    layer = add_to_project(line_layer(*CHAIN, name='scratch'))
    add(dock, DANGLES, layer)
    add(dock, 'line/must-not-overlap-with', layer, layer)
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    assert len(read(path)['layers']) == 1
    assert (
        stored(tmp_path / 'topology_data.gpkg', 'scratch').featureCount() == 2
    )


# The message


@pytest.mark.parametrize('kind', MAKE)
def test_a_message_names_the_layer_and_the_reason(
    dock: Dock, data: Path, tmp_path: Path, bar: t.Any, kind: str
):
    layer = with_roads(dock, data, kind)
    dock.export_rules(tmp_path / 'topology.toml')
    (message,) = info(bar)
    assert kind in message
    assert REASONS[kind] in message
    assert 'topology_data.gpkg' in message
    assert 'roads' not in message
    rollback(layer)


def test_one_message_names_each_layer(
    dock: Dock, data: Path, tmp_path: Path, bar: t.Any
):
    layers = [MAKE[kind](data) for kind in MAKE]
    for layer in layers:
        add(dock, DANGLES, layer)
    dock.export_rules(tmp_path / 'topology.toml')
    (message,) = info(bar)
    for kind, reason in REASONS.items():
        assert kind in message
        assert reason in message
    for layer in layers:
        rollback(layer)


def test_a_layer_with_a_filter_and_edits_gives_one_reason(
    dock: Dock, data: Path, tmp_path: Path, bar: t.Any
):
    layer = narrowed(data)
    assert layer.startEditing()
    feature = QgsFeature(layer.fields())
    feature.setGeometry(QgsGeometry.fromWkt('LINESTRING(0 9, 1 9)'))
    assert layer.addFeature(feature)
    add(dock, DANGLES, layer)
    dock.export_rules(tmp_path / 'topology.toml')
    (message,) = info(bar)
    assert message.count('filtered') == 1
    rollback(layer)


def test_a_successful_export_does_not_warn(
    dock: Dock, data: Path, tmp_path: Path, bar: t.Any
):
    with_roads(dock, data, 'scratch')
    dock.export_rules(tmp_path / 'topology.toml')
    assert problems(bar) == []


# Overwriting


def test_nothing_is_asked_when_the_file_is_new(
    dock: Dock, data: Path, tmp_path: Path, asked: list[str]
):
    with_roads(dock, data, 'scratch')
    dock.export_rules(tmp_path / 'topology.toml')
    assert asked == []


def test_an_existing_file_is_asked_about(
    dock: Dock, data: Path, tmp_path: Path, asked: list[str]
):
    with_roads(dock, data, 'scratch')
    (tmp_path / 'topology_data.gpkg').write_bytes(b'old')
    dock.export_rules(tmp_path / 'topology.toml')
    assert len(asked) == 1
    assert 'topology_data.gpkg' in asked[0]


def test_yes_replaces_the_file(
    dock: Dock, data: Path, tmp_path: Path, asked: list[str]
):
    with_roads(dock, data, 'scratch')
    (tmp_path / 'topology_data.gpkg').write_bytes(b'old')
    dock.export_rules(tmp_path / 'topology.toml')
    assert (
        stored(tmp_path / 'topology_data.gpkg', 'scratch').featureCount() == 2
    )
    assert (tmp_path / 'topology.toml').is_file()


def test_no_leaves_the_file_as_it_was(
    dock: Dock, data: Path, tmp_path: Path, refused: list[str]
):
    with_roads(dock, data, 'scratch')
    (tmp_path / 'topology_data.gpkg').write_bytes(b'old')
    dock.export_rules(tmp_path / 'topology.toml')
    assert len(refused) == 1
    assert (tmp_path / 'topology_data.gpkg').read_bytes() == b'old'


def test_no_writes_no_toml(
    dock: Dock, data: Path, tmp_path: Path, refused: list[str]
):
    with_roads(dock, data, 'scratch')
    (tmp_path / 'topology_data.gpkg').write_bytes(b'old')
    dock.export_rules(tmp_path / 'topology.toml')
    assert not (tmp_path / 'topology.toml').exists()


def test_no_leaves_an_old_toml_as_it_was(
    dock: Dock, data: Path, tmp_path: Path, refused: list[str]
):
    with_roads(dock, data, 'scratch')
    (tmp_path / 'topology_data.gpkg').write_bytes(b'old')
    (tmp_path / 'topology.toml').write_text('# mine\n')
    dock.export_rules(tmp_path / 'topology.toml')
    assert (tmp_path / 'topology.toml').read_text() == '# mine\n'


def test_no_says_that_the_export_was_canceled(
    dock: Dock, data: Path, tmp_path: Path, refused: list[str], bar: t.Any
):
    with_roads(dock, data, 'scratch')
    (tmp_path / 'topology_data.gpkg').write_bytes(b'old')
    dock.export_rules(tmp_path / 'topology.toml')
    assert 'cancel' in message_text(bar).lower()
    assert info(bar) == []


def test_an_existing_file_is_not_asked_about_without_need(
    dock: Dock, data: Path, tmp_path: Path, asked: list[str]
):
    add(dock, DANGLES, add_to_project(saved_layer(data, CHAIN, 'roads')))
    (tmp_path / 'topology_data.gpkg').write_bytes(b'old')
    dock.export_rules(tmp_path / 'topology.toml')
    assert asked == []
    assert (tmp_path / 'topology_data.gpkg').read_bytes() == b'old'


def test_exporting_twice_asks_the_second_time(
    dock: Dock, data: Path, tmp_path: Path, asked: list[str]
):
    with_roads(dock, data, 'scratch')
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    dock.export_rules(path)
    assert len(asked) == 1


def test_the_button_exports_layers_that_are_not_files(
    dock: Dock,
    data: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
):
    from qgis.PyQt.QtWidgets import QFileDialog

    with_roads(dock, data, 'scratch')
    path = tmp_path / 'chosen.toml'
    monkeypatch.setattr(
        QFileDialog, 'getSaveFileName', lambda *a, **k: (str(path), '')
    )
    dock.export_button.click()
    assert path.is_file()
    assert (tmp_path / 'chosen_data.gpkg').is_file()


# What still refuses


def test_a_layer_that_left_the_project_is_named(
    dock: Dock, data: Path, tmp_path: Path, bar: t.Any
):
    gone = add_to_project(saved_layer(data, CHAIN, 'roads'))
    add(dock, DANGLES, gone)
    add(dock, DANGLES, memory_layer(data))
    QgsProject.instance().removeMapLayer(gone.id())
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    assert 'roads' in message_text(bar)
    assert not path.exists()
    assert not (tmp_path / 'topology_data.gpkg').exists()


@pytest.mark.skipif(
    os.name == 'nt' or os.geteuid() == 0, reason='needs a folder that refuses'
)
def test_a_folder_that_cannot_be_written_is_reported(
    dock: Dock, data: Path, tmp_path: Path, bar: t.Any
):
    with_roads(dock, data, 'scratch')
    folder = tmp_path / 'locked'
    folder.mkdir()
    folder.chmod(0o500)
    try:
        dock.export_rules(folder / 'topology.toml')
    finally:
        folder.chmod(0o700)
    assert problems(bar)
    assert not (folder / 'topology.toml').exists()


# Back in the project


def test_the_exported_rules_import_with_the_written_data(
    dock: Dock, data: Path, tmp_path: Path
):
    with_roads(dock, data, 'scratch')
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    QgsProject.instance().clear()
    dock.import_rules(path)
    assert [
        layer.name() for layer in QgsProject.instance().mapLayers().values()
    ] == [
        'roads',
        'scratch',
    ]
    assert len(dock.entries) == 2


# The aliases of the layers


def test_a_layer_is_named_after_its_name_in_the_project(
    dock: Dock, data: Path, tmp_path: Path, executable: str
):
    layer = add_to_project(saved_layer(data, CHAIN, 'water', 'GeoJSON'))
    layer.setName('rivers')
    add(dock, DANGLES, layer)
    click_run(dock)
    expected = [int(text) for text in queue_column(dock, 3) if text]
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    config = read(path)
    assert list(config['layers']) == ['rivers']
    assert config['rules'][0]['layer'] == 'rivers'
    assert Path(config['layers']['rivers']['path']).name == 'water.geojson'
    assert cli_counts(executable, path) == expected


def test_a_renamed_layer_of_a_geopackage_keeps_the_layer_of_the_file(
    dock: Dock, data: Path, tmp_path: Path
):
    layer = add_to_project(saved_layer(data, CHAIN, 'roads'))
    layer.setName('Main roads')
    add(dock, DANGLES, layer)
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    entry = read(path)['layers']['Main roads']
    assert entry['layer'] == 'roads'
    assert Path(entry['path']).as_posix() == 'data/roads.gpkg'


def test_aliases_of_layers_with_the_same_name_are_made_unique(
    dock: Dock, data: Path, tmp_path: Path
):
    first = data / 'a'
    second = data / 'b'
    first.mkdir()
    second.mkdir()
    one = add_to_project(saved_layer(first, CHAIN, 'one', 'GeoJSON'))
    two = add_to_project(saved_layer(second, CHAIN, 'two', 'GeoJSON'))
    one.setName('rivers')
    two.setName('rivers')
    add(dock, DANGLES, one)
    add(dock, DANGLES, two)
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    config = read(path)
    assert len(config['layers']) == 2
    assert 'rivers' in config['layers']
    assert all(alias.startswith('rivers') for alias in config['layers'])
    assert config['rules'][0]['layer'] != config['rules'][1]['layer']


def test_a_layer_of_a_file_and_a_layer_that_is_not_both_use_project_names(
    dock: Dock, data: Path, tmp_path: Path
):
    plain = add_to_project(saved_layer(data, CHAIN, 'water', 'GeoJSON'))
    plain.setName('rivers')
    scratch = add_to_project(line_layer(*CHAIN, name='canals'))
    add(dock, DANGLES, plain)
    add(dock, DANGLES, scratch)
    path = tmp_path / 'topology.toml'
    dock.export_rules(path)
    assert set(read(path)['layers']) == {'rivers', 'canals'}
