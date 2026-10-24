from __future__ import annotations

import dataclasses
import re
import typing as t

from qgis.core import (
    QgsFeature,
    QgsFields,
    QgsGeometry,
    QgsMultiLineString,
    QgsMultiPoint,
    QgsMultiPolygon,
    QgsVectorLayer,
)

from ..exceptions import RunError
from .inputs import not_finite
from .layers import wkb_kind
from .progress import Found

# The columns of the tool, as a memory layer defines them
FIELDS = (
    'violation_id:integer&field=rule:string&field=error_type:string'
    '&field=origin_layer:string&field=origin_fid:integer'
    '&field=origin_part:integer&field=destination_layer:string'
    '&field=destination_fid:integer&field=destination_part:integer'
)
# How the tool names the feature that it rejects; the Python package names
# no layer where it refuses the data that it is given
FAILURE = re.compile(
    r'^(?:layer (?P<layer>.+?), )?FID (?P<fid>-?\d+)'
    r'(?:, part \d+, ring [^:]+)?: (?P<reason>.*)$',
    re.DOTALL,
)
# What it rejects, as opposed to what is wrong with the rule or the layer
# (crates/core/src/error.rs has the messages)
REJECTED = ('invalid polygon', 'coordinates must be finite')
# What the layer of each family holds for a feature that is written without
# a geometry
EMPTY = {
    'point': QgsMultiPoint,
    'line': QgsMultiLineString,
    'polygon': QgsMultiPolygon,
}


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
        # Whether the tool read the input as it is, without leaving out what
        # it would reject: the executable from the layer's own file
        self.as_is = False


def parse_failure(message: str) -> tuple[str | None, int, str] | None:
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

    def found(self) -> Found:
        """The feature as a row of the error layer of its kind, with its
        geometry as the layer stores it: a multi geometry without Z or M
        values, which is empty where a coordinate is not finite."""
        if not_finite(self.geometry):
            geometry = QgsGeometry(EMPTY[self.kind]())
        else:
            geometry = QgsGeometry(self.geometry)
            geometry.convertToMultiType()
            part = geometry.get()
            if part is not None:
                part.dropZValue()
                part.dropMValue()
        # data() is bytes at run time; some stubs type it as str
        wkb = t.cast('bytes', geometry.asWkb().data())
        values = (None, '', 'invalid', self.alias, self.fid)
        return Found(self.kind, wkb, values + (None,) * 4)
