"""Exporting again to the file that the rules were imported from: the layers
that come from its GeoPackage must not be lost to the new export."""

from __future__ import annotations

import typing as t
from pathlib import Path

import pytest
import tomllib
from qgis.core import Qgis, QgsFeature, QgsGeometry, QgsProject, QgsVectorLayer
from qgis.PyQt.QtWidgets import QMessageBox

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    both_engines,
    cli_counts,
    click_run,
    line_layer,
    message_text,
    queue_column,
)
from .test_definition_files import CHAIN, DANGLES

pytestmark = both_engines

COUNT = len(CHAIN)


@pytest.fixture(autouse=True)
def replace_files(monkeypatch: pytest.MonkeyPatch) -> None:
    """Answers Yes when the export asks whether to replace a file."""
    monkeypatch.setattr(
        QMessageBox,
        'question',
        lambda *args, **kwargs: QMessageBox.StandardButton.Yes,
    )


def temporary(name: str) -> QgsVectorLayer:
    layer = line_layer(*CHAIN, name=name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def stored(path: Path, name: str) -> QgsVectorLayer:
    return QgsVectorLayer(f'{path}|layername={name}', name, 'ogr')


@pytest.fixture
def imported(dock: Dock, tmp_path: Path) -> Path:
    """The rules of a temporary layer, exported, then imported in a new
    project: the layer now comes from the GeoPackage of the export."""
    toml = tmp_path / 'topology.toml'
    add(dock, DANGLES, temporary('scratch'))
    dock.export_rules(toml)
    QgsProject.instance().clear()
    dock.import_rules(toml)
    assert [entry.layer_name for entry in dock.entries] == ['scratch']
    return toml


def export_again(dock: Dock, toml: Path) -> None:
    """Adds a rule on another temporary layer, and exports to the same file."""
    add(dock, DANGLES, temporary('extra'))
    dock.export_rules(toml)


def test_the_layer_of_the_data_file_is_still_in_it_after_a_second_export(
    dock: Dock, imported: Path, tmp_path: Path
):
    export_again(dock, imported)
    layer = stored(tmp_path / 'topology_data.gpkg', 'scratch')
    assert layer.isValid()
    assert layer.featureCount() == COUNT


def test_the_project_layer_of_the_data_file_keeps_its_data(
    dock: Dock, imported: Path
):
    layer = t.cast(
        QgsVectorLayer, QgsProject.instance().mapLayersByName('scratch')[0]
    )
    export_again(dock, imported)
    layer.dataProvider().reloadData()
    assert layer.isValid()
    assert layer.featureCount() == COUNT


def test_the_rules_that_the_second_export_wrote_can_be_imported(
    dock: Dock, imported: Path
):
    export_again(dock, imported)
    QgsProject.instance().clear()
    dock.import_rules(imported)
    assert dock.entries
    for entry in dock.entries:
        for layer_id in filter(None, (entry.layer, entry.other)):
            layer = QgsProject.instance().mapLayer(layer_id)
            assert isinstance(layer, QgsVectorLayer)
            assert layer.isValid()
            assert layer.featureCount() == COUNT


def test_an_export_that_leaves_the_file_as_it_was_says_why(
    dock: Dock, imported: Path, bar: t.Any
):
    before = imported.read_text(encoding='utf-8')
    export_again(dock, imported)
    if imported.read_text(encoding='utf-8') == before:
        warned = [
            *bar.get_messages(Qgis.MessageLevel.Warning),
            *bar.get_messages(Qgis.MessageLevel.Critical),
        ]
        assert warned, message_text(bar)


def edit_without_saving(layer: QgsVectorLayer) -> None:
    """Adds a line to the layer in its edit buffer."""
    assert layer.startEditing()
    feature = QgsFeature(layer.fields())
    feature.setGeometry(QgsGeometry.fromWkt('LINESTRING(0 9, 1 9)'))
    assert layer.addFeature(feature)


def scratch_layer() -> QgsVectorLayer:
    return t.cast(
        QgsVectorLayer, QgsProject.instance().mapLayersByName('scratch')[0]
    )


def exported_table(toml: Path) -> tuple[Path, str]:
    """The file and the table that the one layer of the exported rules is
    read from."""
    config = tomllib.loads(toml.read_text(encoding='utf-8'))
    (entry,) = config['layers'].values()
    return toml.parent / entry['path'], entry['layer']


def test_the_edits_are_exported_in_a_new_table(
    dock: Dock, imported: Path, tmp_path: Path
):
    edit_without_saving(scratch_layer())
    dock.export_rules(imported)
    path, table = exported_table(imported)
    assert table != 'scratch'
    layer = stored(path, table)
    assert layer.isValid()
    assert layer.featureCount() == COUNT + 1


def test_the_edits_are_not_saved_into_the_table_that_the_layer_reads(
    dock: Dock, imported: Path, tmp_path: Path
):
    edit_without_saving(scratch_layer())
    dock.export_rules(imported)
    source = stored(tmp_path / 'topology_data.gpkg', 'scratch')
    assert source.isValid()
    assert source.featureCount() == COUNT


def test_the_layer_that_was_edited_keeps_its_features_after_the_export(
    dock: Dock, imported: Path
):
    layer = scratch_layer()
    edit_without_saving(layer)
    dock.export_rules(imported)
    assert layer.isValid()
    assert layer.featureCount() == COUNT + 1
    assert layer.isModified()


def narrow(dock: Dock, how: str) -> None:
    """Makes the layer of the data file count only its first feature: the
    selection, with the option of the selected features, or a filter."""
    layer = scratch_layer()
    if how == 'selected':
        layer.selectByIds([1])
        dock.selected_only.setChecked(True)
    else:
        assert layer.setSubsetString('"fid" = 1')


def widen(how: str) -> None:
    if how == 'filter':
        assert scratch_layer().setSubsetString('')


@pytest.mark.parametrize('how', ['selected', 'filter'])
def test_a_narrow_export_leaves_the_table_that_the_layer_reads_whole(
    dock: Dock, imported: Path, tmp_path: Path, how: str
):
    narrow(dock, how)
    dock.export_rules(imported)
    widen(how)
    layer = stored(tmp_path / 'topology_data.gpkg', 'scratch')
    assert layer.isValid()
    assert layer.featureCount() == COUNT


@pytest.mark.parametrize('how', ['selected', 'filter'])
def test_a_narrow_export_leaves_the_source_of_the_project_layer_whole(
    dock: Dock, imported: Path, how: str
):
    narrow(dock, how)
    dock.export_rules(imported)
    widen(how)
    # Read anew from the source of the layer, as the layer itself reads the
    # file that was replaced under it, which can take QGIS down
    source = QgsVectorLayer(scratch_layer().source(), 'again', 'ogr')
    assert source.isValid()
    assert source.featureCount() == COUNT


@pytest.mark.parametrize('how', ['selected', 'filter'])
def test_a_narrow_export_checks_what_the_panel_checks(
    dock: Dock, imported: Path, executable: str, how: str
):
    narrow(dock, how)
    dock.export_rules(imported)
    exported = cli_counts(executable, imported)
    click_run(dock)
    assert exported == [int(count) for count in queue_column(dock, 3)]


def test_the_rules_exported_with_unsaved_edits_check_the_edits(
    dock: Dock, imported: Path
):
    edit_without_saving(scratch_layer())
    dock.export_rules(imported)
    QgsProject.instance().clear()
    dock.import_rules(imported)
    (entry,) = dock.entries
    layer = QgsProject.instance().mapLayer(entry.layer)
    assert isinstance(layer, QgsVectorLayer)
    assert layer.featureCount() == COUNT + 1


# The file is changed where it is: layers of the project read it


def identity(path: Path) -> tuple[int, int]:
    """Who a file is: replacing it by another file with the same name
    changes this."""
    found = path.stat()
    return found.st_dev, found.st_ino


def test_a_second_export_changes_the_data_file_where_it_is(
    dock: Dock, imported: Path, tmp_path: Path
):
    data = tmp_path / 'topology_data.gpkg'
    before = identity(data)
    export_again(dock, imported)
    assert identity(data) == before


def test_the_layer_of_the_data_file_reads_the_same_file_after_a_second_export(
    dock: Dock, imported: Path, tmp_path: Path
):
    layer = scratch_layer()
    source = layer.source()
    export_again(dock, imported)
    assert layer.source() == source
    assert layer.isValid()
    assert sum(1 for _ in layer.getFeatures()) == COUNT


def test_the_new_layer_is_added_to_the_data_file_beside_the_old_one(
    dock: Dock, imported: Path, tmp_path: Path
):
    export_again(dock, imported)
    data = tmp_path / 'topology_data.gpkg'
    assert stored(data, 'scratch').featureCount() == COUNT
    assert stored(data, 'extra').featureCount() == COUNT
