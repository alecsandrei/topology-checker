from __future__ import annotations

import dataclasses
import math
import os
import typing as t
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsAbstractFeatureSource,
    QgsCoordinateReferenceSystem,
    QgsFeatureSource,
    QgsProject,
    QgsProviderRegistry,
    QgsRectangle,
    QgsUnitTypes,
    QgsVectorLayer,
    QgsVectorLayerFeatureSource,
    QgsWkbTypes,
)

from ..exceptions import RunError
from ..utils import named, not_none, tr
from .binary import readable_extensions

# Formats with several layers per file, which need the layer name. Their
# feature ids are the ones QGIS shows, unlike those of other formats
CONTAINERS = {'.gpkg', '.sqlite'}


@dataclasses.dataclass(frozen=True)
class Dataset:
    path: str
    layer: str | None = None

    @property
    def name(self) -> str:
        """What the dataset is called: its layer, else its file."""
        return self.layer or Path(self.path).stem

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
    needs to know. A run in another thread never reads the layer, which the
    main thread can change meanwhile, but the reader.
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
    # The source of a layer in a GeoPackage, which QGIS may have to open
    # again to read it (see inputs.reconnect)
    shared: str | None = dataclasses.field(init=False)
    crs: QgsCoordinateReferenceSystem = dataclasses.field(init=False)
    # The source gives its features in the order of their ids, which a part
    # of a layer and the new features of an edit session do not
    ordered: bool = dataclasses.field(init=False)
    # What the features are read from. The source of the whole layer, or of
    # the part of it in the ids or the area, reads the layer itself, so the
    # reader is a copy of the layer as it is now, with its unsaved edits
    reader: QgsFeatureSource | QgsAbstractFeatureSource = dataclasses.field(
        init=False
    )

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
            shared=None,
            crs=(layer.crs() if layer is not None else self.source.sourceCrs()),
            ordered=(
                self.whole
                and not self.restricted
                and (layer is None or not added_features(layer))
            ),
            reader=self.source,
        )
        if layer is None or not layer.isValid():
            return
        keep(shared=geopackage_source(layer))
        if self.whole or self.restricted:
            keep(reader=QgsVectorLayerFeatureSource(layer))
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


def geopackage_source(layer: QgsVectorLayer) -> str | None:
    """The source of the provider of a layer in a GeoPackage."""
    provider = layer.dataProvider()
    if (
        layer.providerType() != 'ogr'
        or provider is None
        or provider.storageType() != 'GPKG'
    ):
        return None
    return provider.dataSourceUri()


def private_layer(source: str, name: str) -> QgsVectorLayer:
    """A layer of the OGR source that is not in the project, which a run in
    another thread owns.

    The run reads and edits the layer and never asks for its provider. When
    a run deletes an object that QGIS made and Python wrapped, such as a
    provider, PyQt tells the main thread later. By then another object can
    be at its address, and Python takes that one for deleted.
    """
    options = QgsVectorLayer.LayerOptions(False)
    # No dialog or message about a missing CRS from another thread
    options.skipCrsValidation = True
    return QgsVectorLayer(source, name, 'ogr', options)


def wkb_kind(wkb_type: t.Any) -> str:
    """'point', 'line' or 'polygon', and 'none' for a table."""
    return {0: 'point', 1: 'line', 2: 'polygon'}.get(
        int(QgsWkbTypes.geometryType(wkb_type)), 'none'
    )


def geometry_word(kind: str) -> str:
    """The geometry kind ('point', 'line' or 'polygon') in words."""
    return {
        'point': tr('point'),
        'line': tr('line'),
        'polygon': tr('polygon'),
    }.get(kind, kind)


def described(kind: str) -> str:
    """A layer of the geometry kind, as a message names it."""
    return {
        'any': tr('a layer with geometries'),
        'point': tr('a point layer'),
        'line': tr('a line layer'),
        'polygon': tr('a polygon layer'),
    }.get(kind, tr('a layer without geometries'))


def require_kind(item: Input, kind: str) -> None:
    """Raises if the input is not of the geometry kind ('any' needs one)."""
    found = item.kind
    if found == 'none' or kind not in ('any', found):
        raise RunError(
            tr('{layer} is {found}, but {wanted} is needed.').format(
                layer=item.name, found=described(found), wanted=described(kind)
            )
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
        raise RunError(
            tr('The file of the layer {layer} is gone: {path}').format(
                layer=layer.name(), path=path
            )
        )
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
    of the layer, which it does for the layers of a container. Curves are
    exported as straight segments, which is what the tool reads."""
    if QgsWkbTypes.isCurvedType(layer.wkbType()):
        return None
    found = plain_dataset(layer)
    return found if found is not None and found.layer is not None else None


def project_layer(layer_id: str | None) -> QgsVectorLayer | None:
    if layer_id is None:
        return None
    layer = not_none(QgsProject.instance()).mapLayer(layer_id)
    return layer if isinstance(layer, QgsVectorLayer) else None


# The CRS in which distances are measured


def wkt_parts(wkt: str) -> list[str]:
    """The parts of the outer WKT element, such as the source and target of
    BOUNDCRS[SOURCECRS[...],TARGETCRS[...],...]."""
    parts: list[str] = []
    start = wkt.find('[') + 1
    depth = 0
    quoted = False
    for at in range(start if start else len(wkt), len(wkt)):
        char = wkt[at]
        if char == '"':
            # A quote in a text is doubled, and turns this twice
            quoted = not quoted
        elif quoted:
            continue
        elif char in '[(':
            depth += 1
        elif char in '])' and depth:
            depth -= 1
        elif char in '])' or (char == ',' and not depth):
            parts.append(wkt[start:at].strip())
            if char != ',':
                break
            start = at + 1
    return parts


def horizontal(
    crs: QgsCoordinateReferenceSystem,
) -> QgsCoordinateReferenceSystem:
    """The CRS in which the executable measures distances: the horizontal
    part of a compound CRS, and the source of a bound CRS, which adds a datum
    shift to WGS 84 to it. QGIS takes a bound CRS for neither geographic nor
    projected, and gives no units for a compound CRS with a bound part."""
    scoped = getattr(Qgis, 'CrsWktVariant', None)
    variant = (
        scoped.Wkt2_2019
        if scoped is not None
        else named(QgsCoordinateReferenceSystem, 'WKT2_2019')
    )
    # A bound CRS can be the horizontal part of a compound one
    for _ in range(3):
        if crs.type() == Qgis.CrsType.Bound:
            # BOUNDCRS[SOURCECRS[...],TARGETCRS[...],...]
            source = wkt_parts(crs.toWkt(variant))[:1]
            found = wkt_parts(source[0])[:1] if source else []
        elif crs.type() == Qgis.CrsType.Compound:
            # COMPOUNDCRS["name",horizontal,vertical]
            found = wkt_parts(crs.toWkt(variant))[1:2]
        else:
            break
        part = QgsCoordinateReferenceSystem.fromWkt(found[0]) if found else None
        if part is None or not part.isValid():
            break
        crs = part
    return crs


def geographic(crs: QgsCoordinateReferenceSystem) -> bool:
    """Whether the executable measures distances in degrees in the CRS.
    QGIS takes only a geographic CRS of its database for geographic."""
    found = horizontal(crs)
    return found.isGeographic() or found.type() in (
        Qgis.CrsType.Geographic2d,
        Qgis.CrsType.Geographic3d,
    )


def default_tolerance(crs: QgsCoordinateReferenceSystem) -> float:
    """The tolerance of a run that names none, as the executable has it:
    1e-8 in degrees, a millimetre in a projected CRS, and none in another."""
    if geographic(crs):
        return 1e-8
    found = horizontal(crs)
    units = found.mapUnits()
    if (
        found.type() not in (Qgis.CrsType.Projected, Qgis.CrsType.Compound)
        or QgsUnitTypes.unitType(units) != Qgis.DistanceUnitType.Standard
    ):
        return 0.0
    metres = QgsUnitTypes.fromUnitToUnitFactor(units, Qgis.DistanceUnit.Meters)
    return 0.001 / metres if math.isfinite(metres) and metres > 0 else 0.0
