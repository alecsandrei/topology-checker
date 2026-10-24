from __future__ import annotations

import collections.abc as c
import typing as t
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsCoordinateReferenceSystem,
    QgsCoordinateTransform,
    QgsProject,
    QgsRectangle,
    QgsVectorFileWriter,
    QgsVectorLayer,
)
from qgis.PyQt.QtWidgets import QFileDialog, QMessageBox

from ..core.commands import Check, write_config
from ..core.definition import DefinitionError, read_definition
from ..core.layers import (
    Dataset,
    plain_dataset,
    project_layer,
    same_file,
    uri_parts,
)
from ..exceptions import RunError
from ..utils import not_none, tr
from .entries import Entry, unit_of
from .messages import SECONDS

if t.TYPE_CHECKING:
    from .dock import Dock

# The part of the map that is visible, and its CRS
Area = tuple[QgsRectangle, QgsCoordinateReferenceSystem]


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
            visible_extent(dock) if dock.extent_only.isChecked() else None,
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


def visible_extent(dock: Dock) -> Area:
    """The area of the map canvas, with its CRS."""
    canvas = not_none(dock.iface.mapCanvas())
    return canvas.extent(), canvas.mapSettings().destinationCrs()


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


def layer_area(layer: QgsVectorLayer, extent: Area) -> QgsRectangle:
    """The area in the CRS of the layer."""
    area, crs = extent
    if crs.isValid() and layer.crs().isValid() and crs != layer.crs():
        return QgsCoordinateTransform(
            crs, layer.crs(), not_none(QgsProject.instance())
        ).transformBoundingBox(area)
    return area


def why_saved(
    layer: QgsVectorLayer, readable: bool, selected: bool = True
) -> str:
    """Why a layer is saved in the GeoPackage and not read where it is."""
    if readable:
        return tr('selection') if selected else tr('visible extent')
    if layer.subsetString():
        return tr('filter')
    if layer.isEditable() and layer.isModified():
        return tr('unsaved edits')
    if (
        layer.providerType() == 'ogr'
        and Path(uri_parts(layer).get('path') or '').is_file()
    ):
        return tr('format')
    return tr('temporary layer')


def has_layers(path: Path) -> bool:
    """Whether the file is a GeoPackage, whose layers can be added to."""
    try:
        with path.open('rb') as file:
            return file.read(16) == b'SQLite format 3\0'
    except OSError:
        return False


def tables_read(path: Path) -> dict[str, set[str]]:
    """The tables of the file that layers of the project read, with the ids
    of those layers, whatever is done to a layer (a filter, edits)."""
    found: dict[str, set[str]] = {}
    for layer in not_none(QgsProject.instance()).mapLayers().values():
        if (
            not isinstance(layer, QgsVectorLayer)
            or layer.providerType() != 'ogr'
        ):
            continue
        parts = uri_parts(layer)
        where = str(parts.get('path') or '')
        table = parts.get('layerName')
        if table and where and same_file(where, str(path)):
            found.setdefault(str(table), set()).add(layer.id())
    return found


def write_layer(
    layer: QgsVectorLayer,
    path: Path,
    name: str,
    first: bool,
    selected: bool = False,
    attributes: bool = False,
    area: QgsRectangle | None = None,
) -> None:
    """Writes a layer to the GeoPackage as a table of the name, in a new
    file if it is the first."""
    options = QgsVectorFileWriter.SaveVectorOptions()
    options.driverName = 'GPKG'
    options.fileEncoding = 'UTF-8'
    options.layerName = name
    options.skipAttributeCreation = not attributes
    if not layer.crs().isValid():
        options.layerOptions = ['SRID=-1']
    # Without a selection, nothing: the panel checks nothing there
    options.onlySelectedFeatures = selected
    if area is not None:
        options.filterExtent = area
    options.actionOnExistingFile = (
        QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteFile
        if first
        else QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteLayer
    )
    error, message, *_ = QgsVectorFileWriter.writeAsVectorFormatV3(
        layer,
        str(path),
        not_none(QgsProject.instance()).transformContext(),
        options,
    )
    if error != QgsVectorFileWriter.WriterError.NoError:
        raise DefinitionError(
            tr('Could not save {layer}: {error}').format(
                layer=layer.name(), error=message
            )
        )


def save_layers(
    items: c.Sequence[tuple[QgsVectorLayer, bool, QgsRectangle | None]],
    path: Path,
) -> list[Dataset]:
    """Writes the layers as they are in the project to a GeoPackage, one
    table each, with only the selected features for the ones marked so, and
    returns where each one is.

    A GeoPackage that is there is added to and not replaced, as layers of
    the project may have it open. The tables that layers read are never
    written over, whatever is done to the layer (edits, a filter, a
    selection): the layer is written under a name that is not theirs.
    """
    found: list[Dataset] = []
    adding = has_layers(path)
    taken: set[str] = set(tables_read(path)) if adding else set()
    for layer, selected, area in items:
        name, index = layer.name(), 1
        while name in taken:
            index += 1
            name = f'{layer.name()}_{index}'
        taken.add(name)
        write_layer(layer, path, name, not adding, selected, area=area)
        adding = True
        found.append(Dataset(str(path), name))
    return found


def export_definition(
    entries: list[Entry],
    tolerance: float | None,
    path: Path,
    selected: bool = False,
    overwrite: c.Callable[[Path], bool] = lambda _: True,
    extent: Area | None = None,
) -> dict[str, str]:
    """Writes the checked rules as a topology.toml, which the command line
    also runs, and returns why each layer that is not a file of its own was
    saved in a GeoPackage next to it, by name. Nothing is written if a layer
    is gone from the project.

    As in the panel, the selection and the extent limit the layer that a rule
    checks and not the layer it checks it against."""
    checked = [entry for entry in entries if entry.checked]
    if not checked:
        raise DefinitionError(tr('Check at least one rule to export.'))
    # Each layer with what is asked of it: the selection, or the whole
    limit = selected or extent is not None
    wanted = [((entry.layer, limit), entry.layer_name) for entry in checked]
    wanted += [
        ((entry.other, False), entry.other_name)
        for entry in checked
        if entry.other
    ]
    datasets: dict[tuple[str | None, bool], Dataset] = {}
    unreadable: dict[tuple[str | None, bool], QgsVectorLayer] = {}
    readable: set[tuple[str | None, bool]] = set()
    gone: list[str] = []
    for key, name in wanted:
        layer_id, limited = key
        if layer_id is None or key in datasets or key in unreadable:
            continue
        found = dataset_of(layer_id)
        layer = project_layer(layer_id)
        if found is not None and not limited:
            datasets[key] = found
        elif layer is not None:
            unreadable[key] = layer
            if found is not None:
                readable.add(key)
        else:
            gone.append(name)
    if gone:
        raise DefinitionError(
            tr('These layers are not in the project: {layers}').format(
                layers=', '.join(dict.fromkeys(gone))
            )
        )
    data = data_path(path)
    saved: dict[str, str] = {}
    if unreadable:
        if data.exists() and not overwrite(data):
            raise DefinitionError(tr('Export canceled.'))
        written = save_layers(
            [
                (
                    layer,
                    key[1] and selected,
                    layer_area(layer, extent) if key[1] and extent else None,
                )
                for key, layer in unreadable.items()
            ],
            data,
        )
        datasets.update(zip(unreadable, written, strict=True))
        saved = {
            layer.name(): why_saved(layer, key in readable, selected)
            for key, layer in unreadable.items()
        }
    checks = [
        Check(
            entry.rule,
            datasets[(entry.layer, limit)],
            datasets[(entry.other, False)] if entry.other else None,
        )
        for entry in checked
    ]
    names: dict[Dataset, str] = {}
    for key, name in wanted:
        if key[0] is not None:
            names.setdefault(datasets[key], name)
    write_config(path, checks, tolerance, relative=True, names=names)
    return saved
