"""Exporting again to the file that the rules were imported from: the layers
that come from its GeoPackage must not be lost to the new export."""

from __future__ import annotations

import typing as t
from pathlib import Path

import pytest
from qgis.core import Qgis, QgsFeature, QgsGeometry, QgsProject, QgsVectorLayer
from qgis.PyQt.QtWidgets import QMessageBox

from topology_rules.panel.dock import Dock

from .helpers import add, line_layer, message_text
from .test_definition_files import CHAIN, DANGLES

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


def test_the_data_file_has_the_unsaved_edits_after_the_export(
    dock: Dock, imported: Path, tmp_path: Path
):
    edit_without_saving(scratch_layer())
    dock.export_rules(imported)
    layer = stored(tmp_path / 'topology_data.gpkg', 'scratch')
    assert layer.isValid()
    assert layer.featureCount() == COUNT + 1


def test_the_layer_that_was_edited_keeps_its_features_after_the_export(
    dock: Dock, imported: Path
):
    layer = scratch_layer()
    edit_without_saving(layer)
    dock.export_rules(imported)
    assert layer.isValid()
    assert layer.featureCount() == COUNT + 1
    assert layer.isModified()


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
