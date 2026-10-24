from __future__ import annotations

import collections.abc as c
import dataclasses
import hashlib
import typing as t

from qgis.core import QgsFeature, QgsGeometry, QgsVectorLayer

from ..core.binary import named_with, plain_name, short_name
from ..core.progress import Checked
from ..core.style import INVALID, Grouping, rule_grouping

# The fields of an error that a row is made from
COLUMNS = (
    'rule',
    'error_type',
    'origin_layer',
    'origin_fid',
    'destination_layer',
    'destination_fid',
)


def field_text(feature: QgsFeature, index: int) -> str:
    """The text of the field at the index, empty for a null value or for a
    field that the layer does not have, whose index is -1."""
    if index < 0:
        return ''
    value = feature.attribute(index)
    return '' if value is None or str(value) == 'NULL' else str(value)


def shape_of(feature: QgsFeature) -> str:
    return shape(feature.geometry())


def shape(geometry: QgsGeometry) -> str:
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
    # What the error is told by when the user ignores it, worked out once
    # the layers of the run are known
    mark: str = dataclasses.field(default='', compare=False)
    # The error itself, in the CRS of the run, while it has no error layer
    geometry: QgsGeometry | None = dataclasses.field(
        default=None, compare=False, repr=False
    )

    @property
    def group(self) -> str:
        """The name of the group when the run gave none, such as
        'countries: Must not overlap'."""
        if not self.rule_name:
            return INVALID
        name = short_name(self.rule_name)
        if self.other_alias and self.other_alias != self.layer_alias:
            name = named_with(plain_name(self.rule_name), self.other)
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
        # The fields are looked up once for the layer, not for each feature
        fields = layer.fields()
        indexes = [fields.indexOf(name) for name in COLUMNS]
        for feature in t.cast('c.Iterable[QgsFeature]', layer.getFeatures()):
            rule, error, origin, fid, other, other_fid = (
                field_text(feature, index) for index in indexes
            )
            rows.append(
                Row(
                    rule,
                    error,
                    names.get(origin, origin),
                    number(fid),
                    names.get(other, other),
                    number(other_fid),
                    origin,
                    other,
                    layer.id(),
                    feature.id(),
                    grouping.key_of(feature),
                    shape_of(feature),
                )
            )
    return rows


def value_text(value: object) -> str:
    """A value of an error as field_text reads it from the error layer."""
    return '' if value is None or str(value) == 'NULL' else str(value)


def found_rows(done: Checked, names: c.Mapping[str, str]) -> list[Row]:
    """The errors of a rule that is done, as read_rows reads them from the
    error layers later, with the names of the layers of the project by their
    ids, and the geometry of each error, as there is no layer yet."""
    rows: list[Row] = []
    for found in done.errors:
        _, rule, error, origin, fid, _, other, other_fid, _ = (
            value_text(value) for value in found.values
        )
        rows.append(
            Row(
                rule,
                error,
                names.get(done.layers.get(origin, ''), origin),
                number(fid),
                names.get(done.layers.get(other, ''), other),
                number(other_fid),
                origin,
                other,
                '',
                -1,
                f'e{done.index}',
                shape(found.geometry),
                geometry=found.geometry,
            )
        )
    return rows
