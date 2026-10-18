from __future__ import annotations

import collections.abc as c
import typing as t
from pathlib import Path

from qgis.core import Qgis, QgsProject, QgsVectorFileWriter, QgsVectorLayer
from qgis.PyQt.QtCore import QCoreApplication
from qgis.PyQt.QtWidgets import QFileDialog, QMessageBox

from ..core.commands import Check, write_config
from ..core.definition import DefinitionError, read_definition
from ..core.layers import Dataset, plain_dataset, project_layer
from ..exceptions import RunError
from ..utils import not_none
from .entries import Entry, unit_of
from .messages import SECONDS

if t.TYPE_CHECKING:
    from .dock import Dock


def tr(text: str) -> str:
    return QCoreApplication.translate('TopologyRules', text)


def export_rules(dock: Dock, path: Path) -> None:
    """Writes the checked rules as a topology.toml. The layers that are not
    files of their own are saved in a GeoPackage next to it."""

    def overwrite(data: Path) -> bool:
        return (
            QMessageBox.question(
                dock,
                tr('Export rules'),
                tr('{name} exists. Replace it?').format(name=data.name),
            )
            == QMessageBox.StandardButton.Yes
        )

    try:
        saved = export_definition(
            dock.entries,
            dock.tolerance_value(),
            Path(path),
            dock.selected_only.isChecked(),
            overwrite,
        )
    except DefinitionError as error:
        dock.warn(str(error))
        return
    if saved:
        not_none(dock.iface.messageBar()).pushMessage(
            tr('Topology Rules'),
            tr('Saved in {name}: {layers}').format(
                name=data_path(Path(path)).name,
                layers=', '.join(f'{n} ({why})' for n, why in saved.items()),
            ),
            Qgis.MessageLevel.Info,
            SECONDS,
        )


def import_rules(dock: Dock, path: Path) -> None:
    """Adds the rules of a topology.toml, and its layers that the project
    lacks. A file that cannot be used changes nothing."""
    try:
        found = read_definition(
            Path(path), {rule.name: rule for rule in dock.form.rules}
        )
    except DefinitionError as error:
        dock.warn(str(error))
        return
    project = not_none(QgsProject.instance())
    for layer in found.new_layers:
        project.addMapLayer(layer)
    for rule, layer, other in found.rules:
        dock.rule_list.add(
            Entry(
                rule,
                layer.id(),
                layer.name(),
                unit_of(layer),
                None if other is None else other.id(),
                '' if other is None else other.name(),
            )
        )
    if found.tolerance is not None:
        dock.tolerance.setValue(found.tolerance)


def ask_export(dock: Dock) -> None:
    path, _ = QFileDialog.getSaveFileName(
        dock, tr('Export rules'), 'topology.toml', 'TOML (*.toml)'
    )
    if path:
        export_rules(dock, Path(path))


def ask_import(dock: Dock) -> None:
    path, _ = QFileDialog.getOpenFileName(
        dock, tr('Import rules'), '', 'TOML (*.toml)'
    )
    if path:
        import_rules(dock, Path(path))


def dataset_of(layer_id: str | None) -> Dataset | None:
    layer = project_layer(layer_id)
    if layer is None:
        return None
    try:
        return plain_dataset(layer)
    except RunError:
        return None


def data_path(config: Path) -> Path:
    return config.with_name(f'{config.stem}_data.gpkg')


def why_unreadable(layer: QgsVectorLayer) -> str:
    if layer.subsetString():
        return tr('filter')
    if layer.isEditable() and layer.isModified():
        return tr('unsaved edits')
    return tr('temporary layer')


def save_layers(
    layers: c.Sequence[QgsVectorLayer], path: Path, selected: bool
) -> dict[str, Dataset]:
    """Writes the layers as they are in the project to a GeoPackage, one
    layer each, and returns where each one is."""
    found: dict[str, Dataset] = {}
    taken: set[str] = set()
    transform = not_none(QgsProject.instance()).transformContext()
    for layer in layers:
        name, index = layer.name(), 1
        while name in taken:
            index += 1
            name = f'{layer.name()}_{index}'
        taken.add(name)
        options = QgsVectorFileWriter.SaveVectorOptions()
        options.driverName = 'GPKG'
        options.fileEncoding = 'UTF-8'
        options.layerName = name
        options.skipAttributeCreation = True
        options.onlySelectedFeatures = selected and bool(
            layer.selectedFeatureIds()
        )
        options.actionOnExistingFile = (
            QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteLayer
            if found
            else QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteFile
        )
        error, message, *_ = QgsVectorFileWriter.writeAsVectorFormatV3(
            layer, str(path), transform, options
        )
        if error != QgsVectorFileWriter.WriterError.NoError:
            raise DefinitionError(f'Could not save {layer.name()}: {message}')
        found[layer.id()] = Dataset(str(path), name)
    return found


def export_definition(
    entries: list[Entry],
    tolerance: float | None,
    path: Path,
    selected: bool = False,
    overwrite: c.Callable[[Path], bool] = lambda _: True,
) -> dict[str, str]:
    """Writes the checked rules as a topology.toml, which the command line
    also runs, and returns why each layer that is not a file of its own was
    saved in a GeoPackage next to it, by name. Nothing is written if a layer
    is gone from the project."""
    checked = [entry for entry in entries if entry.checked]
    if not checked:
        raise DefinitionError('Check at least one rule to export.')
    wanted = [(entry.layer, entry.layer_name) for entry in checked] + [
        (entry.other, entry.other_name) for entry in checked if entry.other
    ]
    datasets: dict[str, Dataset] = {}
    unreadable: dict[str, QgsVectorLayer] = {}
    gone: list[str] = []
    for layer_id, name in wanted:
        if layer_id is None or layer_id in datasets or layer_id in unreadable:
            continue
        if (found := dataset_of(layer_id)) is not None:
            datasets[layer_id] = found
        elif (layer := project_layer(layer_id)) is not None:
            unreadable[layer_id] = layer
        else:
            gone.append(name)
    if gone:
        raise DefinitionError(
            'These layers are not in the project: '
            + ', '.join(dict.fromkeys(gone))
        )
    data = data_path(path)
    saved: dict[str, str] = {}
    if unreadable:
        if data.exists() and not overwrite(data):
            raise DefinitionError('Export canceled.')
        datasets.update(save_layers(list(unreadable.values()), data, selected))
        saved = {
            layer.name(): why_unreadable(layer) for layer in unreadable.values()
        }
    checks = [
        Check(
            entry.rule,
            datasets[entry.layer],
            datasets[entry.other] if entry.other else None,
        )
        for entry in checked
    ]
    names: dict[Dataset, str] = {}
    for layer_id, name in wanted:
        if layer_id is not None:
            names.setdefault(datasets[layer_id], name)
    write_config(path, checks, tolerance, relative=True, names=names)
    return saved
