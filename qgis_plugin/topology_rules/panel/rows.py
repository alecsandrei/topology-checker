from __future__ import annotations

import collections.abc as c
import dataclasses
import hashlib
import sys

from ..core.binary import named_with, plain_name, short_name
from ..core.progress import Checked
from ..core.style import INVALID

# The geometries of the errors of a rule as WKB, by the family of their
# layer: the id of the first, and the WKB of each in the order of the ids
Shapes = dict[str, tuple[int, tuple[bytes, ...]]]


def wkb_shape(wkb: bytes) -> str:
    """Tells a geometry, as WKB, from another one; empty for no geometry."""
    return hashlib.blake2b(wkb, digest_size=8).hexdigest() if wkb else ''


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


@dataclasses.dataclass(frozen=True, slots=True)
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
    # Where the error itself is: the error layer of its family, such as
    # 'line', and its id there
    family: str
    error_fid: int
    # The group of the error, which is the rule of the list that found it
    key: str = ''
    # Tells the geometry of the error from another one
    shape: str = ''
    # What the error is told by when the user ignores it, worked out once
    # the layers of the run are known
    mark: str = dataclasses.field(default='', compare=False)

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


def value_text(value: object) -> str:
    """A value of an error as text, empty for none."""
    return '' if value is None or str(value) == 'NULL' else str(value)


def shared(*texts: str) -> tuple[str, ...]:
    """The texts that many errors have, such as the rule, the kind and the
    layers, as one string each for all the errors."""
    return tuple(map(sys.intern, texts))


def found_rows(
    done: Checked, names: c.Mapping[str, str], key: str
) -> list[Row]:
    """The errors of a rule that is done, or the features that were left
    out of the run, as the tree shows them in the group of the key, with the
    names of the layers of the project by their ids. Each error has the id
    that it gets in the layer of its family, after those that the layer has
    before it."""
    rows: list[Row] = []
    fids = {**done.written}
    for found in done.errors:
        (
            _violation,
            rule,
            error,
            origin,
            fid,
            _part,
            other,
            other_fid,
            _other_part,
        ) = (value_text(value) for value in found.values)
        rule, error, origin, other = shared(rule, error, origin, other)
        fids[found.family] = fids.get(found.family, 0) + 1
        rows.append(
            Row(
                rule_name=rule,
                error=error,
                layer=names.get(done.layers.get(origin, ''), origin),
                feature=number(fid),
                other=names.get(done.layers.get(other, ''), other),
                other_feature=number(other_fid),
                layer_alias=origin,
                other_alias=other,
                family=found.family,
                error_fid=fids[found.family],
                key=key,
                shape=wkb_shape(found.wkb),
            )
        )
    return rows


def found_shapes(done: Checked) -> Shapes:
    """The geometries of the errors of a rule that is done, to zoom to
    while they have no layer."""
    found: dict[str, list[bytes]] = {}
    for item in done.errors:
        found.setdefault(item.family, []).append(item.wkb)
    return {
        family: (done.written.get(family, 0) + 1, tuple(wkbs))
        for family, wkbs in found.items()
    }
