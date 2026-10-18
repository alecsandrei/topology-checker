from __future__ import annotations

import collections.abc as c
import dataclasses
import hashlib
import typing as t

from qgis.core import QgsFeature, QgsVectorLayer

from ..core.binary import named_with, short_name
from ..core.style import INVALID, Grouping, rule_grouping


def field_text(layer: QgsVectorLayer, feature: QgsFeature, name: str) -> str:
    if layer.fields().indexOf(name) < 0:
        return ''
    value = feature[name]
    return '' if value is None or str(value) == 'NULL' else str(value)


def shape_of(feature: QgsFeature) -> str:
    geometry = feature.geometry()
    if geometry.isNull():
        return ''
    # data() is bytes at run time; some stubs type it as str
    wkb = t.cast('bytes', geometry.asWkb().data())
    return hashlib.blake2b(wkb, digest_size=8).hexdigest()


def number(text: str) -> int | None:
    return int(text) if text.lstrip('-').isdigit() else None


def humanize(error: str) -> str:
    """'outer-boundary' as 'Outer boundary'."""
    text = error.replace('-', ' ').replace('_', ' ')
    return text[:1].upper() + text[1:]


def plural(error: str) -> str:
    """The name of a kind of error for a group of them: 'Dangles',
    'Outer boundaries'. A kind that is not a thing, such as 'Not covered',
    stays as it is."""
    text = humanize(error)
    word = text.rpartition(' ')[2]
    if word.endswith(('ed', 'ing', 'ive', 'ous', 's')):
        return text
    if word.endswith('y') and word[-2:-1] not in 'aeiou':
        return text[:-1] + 'ies'
    if word.endswith(('x', 'ch', 'sh')):
        return text + 'es'
    return text + 's'


@dataclasses.dataclass(frozen=True)
class Row:
    """An error, as the tree shows it."""

    # The rule as the tool names it, which is empty for invalid geometries
    rule_name: str
    error: str
    layer: str
    feature: int | None
    other: str
    other_feature: int | None
    # The names of the layers in the errors, which the run gave
    layer_alias: str
    other_alias: str
    # Where the error itself is
    error_layer: str
    error_fid: int
    # The group of the error, which is the rule of the list that found it
    key: str = ''
    # Tells the geometry of the error from another one
    shape: str = ''

    @property
    def group(self) -> str:
        """The name of the group when the run gave none, such as
        'countries: Must not overlap'."""
        if not self.rule_name:
            return INVALID
        name = short_name(self.rule_name)
        if self.other_alias and self.other_alias != self.layer_alias:
            name = named_with(name, self.other)
        return f'{self.layer}: {name}'

    @property
    def features(self) -> str:
        """The features that the error is about, such as 'rivers 12' or
        'lakes 6 · countries 168'."""
        parts = [
            f'{layer} {fid}'.strip()
            for layer, fid in (
                (self.layer, self.feature),
                (self.other, self.other_feature),
            )
            if fid is not None
        ]
        return ' · '.join(parts)


def read_rows(
    layers: c.Iterable[QgsVectorLayer],
    names: dict[str, str],
    grouping: Grouping | None = None,
) -> list[Row]:
    """The errors of the error layers, with the names of the layers of the
    project for the aliases of the run."""
    grouping = grouping or rule_grouping()
    rows: list[Row] = []
    for layer in layers:
        for feature in t.cast('c.Iterable[QgsFeature]', layer.getFeatures()):
            origin = field_text(layer, feature, 'origin_layer')
            other = field_text(layer, feature, 'destination_layer')
            rows.append(
                Row(
                    field_text(layer, feature, 'rule'),
                    field_text(layer, feature, 'error_type'),
                    names.get(origin, origin),
                    number(field_text(layer, feature, 'origin_fid')),
                    names.get(other, other),
                    number(field_text(layer, feature, 'destination_fid')),
                    origin,
                    other,
                    layer.id(),
                    feature.id(),
                    grouping.key_of(feature),
                    shape_of(feature),
                )
            )
    return rows
