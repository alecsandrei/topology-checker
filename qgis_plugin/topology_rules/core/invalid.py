from __future__ import annotations

import dataclasses
import re
import typing as t
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsCoordinateReferenceSystem,
    QgsFeature,
    QgsFields,
    QgsGeometry,
    QgsProcessingContext,
    QgsVectorFileWriter,
    QgsVectorLayer,
)

from ..exceptions import RunError
from ..utils import not_none
from .inputs import not_finite
from .layers import wkb_kind

# The columns of the tool, as a memory layer defines them
FIELDS = (
    'violation_id:integer&field=rule:string&field=error_type:string'
    '&field=origin_layer:string&field=origin_fid:integer'
    '&field=origin_part:integer&field=destination_layer:string'
    '&field=destination_fid:integer&field=destination_part:integer'
)
# The geometry that each layer of errors holds
MULTI = {
    'point': 'MultiPoint',
    'line': 'MultiLineString',
    'polygon': 'MultiPolygon',
}
# How the tool names the feature that it rejects
FAILURE = re.compile(
    r'^layer (?P<layer>.+?), FID (?P<fid>-?\d+)'
    r'(?:, part \d+, ring [^:]+)?: (?P<reason>.*)$',
    re.DOTALL,
)
# What it rejects, as opposed to what is wrong with the rule or the layer
# (crates/core/src/error.rs has the messages)
REJECTED = ('invalid polygon', 'coordinates must be finite')


def error_fields() -> QgsFields:
    return QgsVectorLayer(f'None?field={FIELDS}', 'fields', 'memory').fields()


class Rejected(RunError):
    """The tool refused a feature, which is named by its source id."""

    def __init__(self, name: str, fid: int, reason: str, key: object):
        super().__init__(f'{name}, feature {fid}: {reason}')
        self.fid = fid
        self.reason = reason
        self.key = key
        # What the tool said, for the features it can be told to skip
        self.skippable = reason.startswith(REJECTED)
        # Whether the tool read the layer from its own file, where the
        # number of the feature is not told apart from the export's
        self.direct = False


def parse_failure(message: str) -> tuple[str, int, str] | None:
    """The layer, feature and reason in a message of the tool about a
    feature."""
    found = FAILURE.match(message.strip())
    if found is None:
        return None
    return found['layer'], int(found['fid']), found['reason'].strip()


@dataclasses.dataclass(frozen=True)
class Invalid:
    """A feature that was left out of a run, which is an error of its own."""

    alias: str
    fid: int
    geometry: QgsGeometry
    reason: str

    @property
    def kind(self) -> str:
        return wkb_kind(self.geometry.wkbType())

    def feature(self, fields: QgsFields) -> QgsFeature:
        feature = QgsFeature(fields)
        # A coordinate that is not finite cannot be in a GeoPackage, where it
        # breaks the extent of the layer: the error is listed without a
        # geometry, and there is nothing to zoom to
        if not not_finite(self.geometry):
            geometry = QgsGeometry(self.geometry)
            geometry.convertToMultiType()
            feature.setGeometry(geometry)
        values = {
            'rule': '',
            'error_type': 'invalid',
            'origin_layer': self.alias,
            'origin_fid': self.fid,
        }
        feature.setAttributes([values.get(n) for n in fields.names()])
        return feature


def append_invalid(
    path: str,
    invalid: t.Sequence[Invalid],
    crs: QgsCoordinateReferenceSystem,
    context: QgsProcessingContext,
) -> None:
    """Adds the invalid features to the errors of a GeoPackage, which the
    tool has not made when it found no other errors."""
    fields = error_fields()
    for kind in MULTI:
        found = [item for item in invalid if item.kind == kind]
        if not found:
            continue
        name = f'errors_{kind}'
        layer = QgsVectorLayer(f'{path}|layername={name}', name, 'ogr')
        if layer.isValid():
            added, _ = not_none(layer.dataProvider()).addFeatures(
                [item.feature(layer.fields()) for item in found]
            )
            if not added:
                raise RunError(f'Could not write the {kind} errors')
            continue
        features = [item.feature(fields) for item in found]
        options = QgsVectorFileWriter.SaveVectorOptions()
        options.driverName = 'GPKG'
        options.fileEncoding = 'UTF-8'
        options.layerName = name
        if Path(path).exists():
            options.actionOnExistingFile = (
                QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteLayer
            )
        writer = QgsVectorFileWriter.create(
            path,
            fields,
            getattr(Qgis.WkbType, MULTI[kind]),
            crs,
            context.transformContext(),
            options,
        )
        if writer is None:
            raise RunError(f'Could not write the {kind} errors')
        res = writer.addFeatures(features)
        ok = res[0] if isinstance(res, tuple) else bool(res)
        if not ok:
            raise RunError(f'Could not write the {kind} errors')
        del writer
