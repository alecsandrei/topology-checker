from __future__ import annotations

import dataclasses
import os
import typing as t
from pathlib import Path

from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsFeatureSource,
    QgsProject,
    QgsProviderRegistry,
    QgsRectangle,
    QgsVectorLayer,
    QgsWkbTypes,
)

from ..exceptions import RunError
from ..utils import not_none
from .binary import readable_extensions

# Formats with several layers per file, which need the layer name. Their
# feature ids are the ones QGIS shows, unlike those of other formats
CONTAINERS = {'.gpkg', '.sqlite'}


@dataclasses.dataclass(frozen=True)
class Dataset:
    path: str
    layer: str | None = None

    def arguments(self, layer_flag: str) -> list[str]:
        args = [self.path]
        if self.layer is not None:
            args += [layer_flag, self.layer]
        return args


@dataclasses.dataclass(frozen=True)
class Input:
    """What a rule reads from the project: the features that were chosen.

    The layer is behind the source when there is one. The source is the
    layer as the user sees it, with its filter and unsaved edits, and with
    the selection or the limit that was asked for.

    An input is made on the main thread, where it asks the layer what a run
    needs to know. A run in another thread only reads the source, which is
    made for that, and never the layer.
    """

    name: str
    source: QgsFeatureSource
    layer: QgsVectorLayer | None = None
    # No selection, limit or filter was asked for
    whole: bool = True
    # Only these features, by their ids in the source, and only the ones in
    # this area (in the CRS of the source) are to be checked
    ids: frozenset[int] | None = None
    area: QgsRectangle | None = None
    # Who the input is, to read a layer once for all the rules that use it
    key: object = dataclasses.field(init=False)
    # The id of the layer in the project, kept for a run in another thread,
    # which must not ask the layer
    layer_id: str | None = dataclasses.field(init=False)
    valid: bool = dataclasses.field(init=False)
    # The file behind the layer, and why it is not there if it is gone
    path: str | None = dataclasses.field(init=False)
    gone: str = dataclasses.field(init=False)
    # The layer in its own file, if the tool can read it from there
    direct: Dataset | None = dataclasses.field(init=False)
    crs: QgsCoordinateReferenceSystem = dataclasses.field(init=False)
    # The source gives its features in the order of their ids, which a part
    # of a layer and the new features of an edit session do not
    ordered: bool = dataclasses.field(init=False)

    def __post_init__(self) -> None:
        layer = self.layer

        def keep(**values: t.Any) -> None:
            for name, value in values.items():
                object.__setattr__(self, name, value)

        keep(
            layer_id=None if layer is None else layer.id(),
            key=(
                layer.id()
                if self.whole and layer is not None and not self.restricted
                else id(self.source)
            ),
            valid=layer is None or layer.isValid(),
            path=None,
            gone='',
            direct=None,
            crs=(layer.crs() if layer is not None else self.source.sourceCrs()),
            ordered=(
                self.whole
                and not self.restricted
                and (layer is None or not added_features(layer))
            ),
        )
        if layer is None or not layer.isValid():
            return
        try:
            keep(path=layer_file(layer))
            if self.whole and not self.restricted:
                keep(direct=file_dataset(layer))
        except RunError as error:
            keep(gone=str(error))

    @property
    def restricted(self) -> bool:
        """Whether only a part of the features is to be checked."""
        return self.ids is not None or self.area is not None

    @property
    def kind(self) -> str:
        return wkb_kind(self.source.wkbType())


def added_features(layer: QgsVectorLayer) -> bool:
    """Whether the layer has features that were added and not saved."""
    buffer = layer.editBuffer()
    return buffer is not None and bool(buffer.addedFeatures())


def wkb_kind(wkb_type: t.Any) -> str:
    """'point', 'line' or 'polygon', and 'none' for a table."""
    return {0: 'point', 1: 'line', 2: 'polygon'}.get(
        int(QgsWkbTypes.geometryType(wkb_type)), 'none'
    )


def require_kind(item: Input, kind: str) -> None:
    """Raises if the input is not of the geometry kind ('any' needs one)."""
    found = item.kind
    if found == 'none' or kind not in ('any', found):
        wanted = (
            'a layer with geometries' if kind == 'any' else f'a {kind} layer'
        )
        raise RunError(
            f'{item.name} is a {found} layer, but {wanted} is needed.'
        )


def same_file(first: str, second: str) -> bool:
    try:
        return os.path.samefile(first, second)
    except OSError:
        return Path(first).resolve() == Path(second).resolve()


def uri_parts(layer: QgsVectorLayer) -> dict[str, t.Any]:
    """What the OGR provider says the source of a layer is made of."""
    return t.cast(
        'dict[str, t.Any]',
        not_none(QgsProviderRegistry.instance()).decodeUri(
            'ogr', layer.source()
        ),
    )


def layer_file(layer: QgsVectorLayer) -> str | None:
    """The file behind an OGR layer, raising if it was removed."""
    if layer.providerType() != 'ogr':
        return None
    path = str(uri_parts(layer).get('path') or '')
    if path and Path(path).suffix and not Path(path).exists():
        raise RunError(f'The file of the layer {layer.name()} is gone: {path}')
    return path if path and Path(path).is_file() else None


def plain_dataset(layer: QgsVectorLayer) -> Dataset | None:
    """The file behind a layer, if the tool can read all of the layer from
    it, whatever the format."""
    path = layer_file(layer)
    if path is None or layer.subsetString():
        return None
    # The file holds what was saved, not the edits of the layer
    if layer.isEditable() and layer.isModified():
        return None
    # A file inside an archive or behind a URL is not a file of its own
    parts = uri_parts(layer)
    if parts.get('vsiPrefix') or parts.get('vsiSuffix') or '/vsi' in path:
        return None
    suffix = Path(path).suffix.lower()
    # The tool reads some formats only, and no layer of a file but the
    # first: a KML file is several layers to QGIS, one to the tool
    if suffix.lstrip('.') not in readable_extensions() or suffix in {
        '.kml',
        '.kmz',
    }:
        return None
    if suffix in CONTAINERS:
        name = parts.get('layerName')
        return Dataset(path, str(name)) if name else None
    return Dataset(path)


def file_dataset(layer: QgsVectorLayer) -> Dataset | None:
    """The file behind a layer, if the tool also gives the features the ids
    of the layer, which it does for the layers of a container."""
    found = plain_dataset(layer)
    return found if found is not None and found.layer is not None else None


def project_layer(layer_id: str | None) -> QgsVectorLayer | None:
    if layer_id is None:
        return None
    layer = not_none(QgsProject.instance()).mapLayer(layer_id)
    return layer if isinstance(layer, QgsVectorLayer) else None
