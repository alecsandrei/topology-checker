"""The report of the executable, read as it comes.

With `--json validate --stream` the executable says the report of each rule
on a line of its own as soon as the rule is done, in the order of the rules,
then a line with the tolerance and the warnings. An older one says the
whole report on one line at the end, whose `rules` are the same reports.
Each error has its geometry in the form of geo-types, which is made WKB
here.

Each report names its rule and layers, which have to be those of the next
rule of the run: a run whose report is of other rules, or of more, fails.
"""

from __future__ import annotations

import collections.abc as c
import json
import struct
import typing as t

from ..exceptions import RunError
from ..utils import tr
from .errors import Gathered, found_in, from_wkb
from .progress import Found

# The ids of the source features of each exported layer, by its alias: the
# n-th feature of the export came from the n-th id
IdMap = dict[str, tuple[int, ...]]
# The alias of the layer of each rule of the run, and of its other layer
Names = c.Sequence[tuple[str, 'str | None']]

POINT, LINE_STRING, POLYGON = 1, 2, 3
MULTI_POINT, MULTI_LINE_STRING, MULTI_POLYGON, COLLECTION = 4, 5, 6, 7
# The start of each geometry: little-endian, and its type
HEADERS = {kind: struct.pack('<BI', 1, kind) for kind in range(1, 8)}
# What a report that cannot be read raises
UNREADABLE = (
    KeyError,
    TypeError,
    ValueError,
    AttributeError,
    IndexError,
    OverflowError,
    RecursionError,
    struct.error,
)

Coord = t.Mapping[str, float]


def source_id(ids: IdMap, layer: object, fid: object) -> object:
    """The id of the source feature that an error names in a layer."""
    table = ids.get(str(layer))
    if table is None or not isinstance(fid, int):
        return fid
    return table[fid - 1] if 1 <= fid <= len(table) else fid


def coords(points: c.Sequence[Coord], out: bytearray) -> None:
    flat = [value for point in points for value in (point['x'], point['y'])]
    out += struct.pack(f'<I{len(flat)}d', len(points), *flat)


def polygon(rings: c.Sequence[c.Sequence[Coord]], out: bytearray) -> None:
    out += HEADERS[POLYGON] + struct.pack('<I', len(rings))
    for ring in rings:
        coords(ring, out)


def collection(
    kind: int, items: c.Sequence[t.Any], out: bytearray
) -> c.Sequence[t.Any]:
    """Starts a collection of the items, which are written next."""
    out += HEADERS[kind] + struct.pack('<I', len(items))
    return items


def write(geometry: object, out: bytearray) -> None:
    """Writes the WKB of a geometry as geo-types has it in JSON, such as
    {"Point": {"x": 1.0, "y": 2.0}}. A line, a rectangle and a triangle are
    the line string and the polygons that they stand for."""
    ((kind, value),) = t.cast('dict[str, t.Any]', geometry).items()
    if kind == 'Point':
        out += HEADERS[POINT] + struct.pack('<2d', value['x'], value['y'])
    elif kind == 'Line':
        out += HEADERS[LINE_STRING]
        coords([value['start'], value['end']], out)
    elif kind == 'LineString':
        out += HEADERS[LINE_STRING]
        coords(value, out)
    elif kind == 'Polygon':
        polygon([value['exterior'], *value['interiors']], out)
    elif kind == 'MultiPoint':
        for point in collection(MULTI_POINT, value, out):
            out += HEADERS[POINT] + struct.pack('<2d', point['x'], point['y'])
    elif kind == 'MultiLineString':
        for line in collection(MULTI_LINE_STRING, value, out):
            out += HEADERS[LINE_STRING]
            coords(line, out)
    elif kind == 'MultiPolygon':
        for part in collection(MULTI_POLYGON, value, out):
            polygon([part['exterior'], *part['interiors']], out)
    elif kind == 'GeometryCollection':
        for member in collection(COLLECTION, value, out):
            write(member, out)
    elif kind == 'Rect':
        low, high = value['min'], value['max']
        corners = [
            (high['x'], low['y']),
            (high['x'], high['y']),
            (low['x'], high['y']),
            (low['x'], low['y']),
            (high['x'], low['y']),
        ]
        polygon([[{'x': x, 'y': y} for x, y in corners]], out)
    elif kind == 'Triangle':
        first, second, third = value
        polygon([[first, second, third, first]], out)
    else:
        raise ValueError(kind)


def wkb(geometry: object) -> bytes:
    out = bytearray()
    write(geometry, out)
    return bytes(out)


def number(value: object) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(value)
    return value


def text(value: object) -> str:
    if not isinstance(value, str):
        raise TypeError(value)
    return value


def shown(rule: str, layers: c.Iterable[str]) -> str:
    """A rule and its layers, for the log: 'line/must-not-intersect-with
    (roads, rivers)'."""
    return f'{rule} ({", ".join(layers)})'


class Report:
    """Reads the lines of a run of the executable. The errors of each rule
    are gathered as soon as its line is read; the warnings are those of the
    last line. The names are the aliases of the layers of each rule."""

    def __init__(self, gathered: Gathered, ids: IdMap, names: Names) -> None:
        self.gathered = gathered
        self.ids = ids
        self.names = names
        # The number of the next error, over all rules
        self.violation = 0
        self.warnings: list[str] | None = None
        # The number that the tool gives each layer, by its alias
        self.numbers: dict[str, int] = {}

    @property
    def complete(self) -> bool:
        """Whether every rule and the warnings were read."""
        return (
            len(self.gathered.counts) == len(self.gathered.jobs)
            and self.warnings is not None
        )

    def line(self, line: str) -> bool:
        """Reads a line of the output, and says whether it was a line of the
        report. One that is not, such as the start of a line that was cut
        off, says nothing of the run, nor does any line once the report is
        complete."""
        if self.complete:
            return False
        try:
            value = json.loads(line)
        except (ValueError, RecursionError):
            return False
        if not isinstance(value, dict):
            return False
        entry = t.cast('dict[str, object]', value)
        if 'spec' in entry:
            return self.rule(entry)
        if 'tolerance' not in entry:
            return False
        # An older executable has the rules in the report
        rules = entry.get('rules', [])
        if not isinstance(rules, list):
            return False
        read = all(self.rule(rule) for rule in t.cast('list[object]', rules))
        return self.summary(entry) and read

    def rule(self, value: object) -> bool:
        """Reads the report of the next rule, which is gathered once all of
        it is read. One that cannot be read is not gathered. One of another
        rule or other layers than the next rule, or past the last, fails
        the run, which keeps the rules before it."""
        violation = self.violation
        try:
            entry = t.cast('dict[str, t.Any]', value)
            spec = entry['spec']
            layers = [(text(entry['layer']), number(spec['layer']))]
            if (spec['other'] is None) != (entry['other'] is None):
                raise ValueError(entry['other'])
            if spec['other'] is not None:
                layers.append((text(entry['other']), number(spec['other'])))
            numbers = self.expect(text(spec['rule']), layers)
            aliases = {index: name for name, index in layers}
            count = sum(number(n) for n in entry['counts'].values())
            found: list[Found] = []
            for error in entry['errors']:
                # The run stops at once, without this rule
                if self.gathered.feedback.isCanceled():
                    return False
                values = (
                    violation,
                    str(error['rule']),
                    str(error['error_type']),
                    *self.feature(error['origin'], aliases),
                    *self.feature(error['destination'], aliases),
                )
                found += found_in(from_wkb(wkb(error['geometry'])), values)
                violation += 1
        except UNREADABLE:
            return False
        self.violation = violation
        self.numbers = numbers
        self.gathered.add(found, count)
        return True

    def expect(
        self, rule: str, layers: c.Sequence[tuple[str, int]]
    ) -> dict[str, int]:
        """The numbers of the layers once a report of the rule is read, with
        the aliases and the numbers of its layers. Raises unless it is the
        next rule of the run, on its layers, and the tool numbers each layer
        as it did before."""
        jobs = self.gathered.jobs
        index = len(self.gathered.counts)
        numbers = dict(self.numbers)
        named = {given: name for name, given in numbers.items()}
        renumbered = False
        for name, given in layers:
            had = numbers.setdefault(name, given)
            renumbered |= had != given or named.setdefault(given, name) != name
        told = shown(rule, (f'{name} #{given}' for name, given in layers))
        if index < len(jobs):
            expected = (
                jobs[index].rule.name,
                [name for name in self.names[index] if name is not None],
            )
            if (rule, [name for name, _ in layers]) == expected and not (
                renumbered
            ):
                return numbers
            detail = tr(
                'Rule {number} of the run is {expected}, and the tool told '
                'of {rule}'
            ).format(number=index + 1, expected=shown(*expected), rule=told)
        else:
            detail = tr(
                'The run has {count} rules, and the tool told of {rule} too'
            ).format(count=len(jobs), rule=told)
        self.gathered.feedback.pushInfo(detail)
        raise RunError(
            tr('The report of the tool is not of the rules of the run')
        )

    def feature(
        self, reference: t.Any, aliases: c.Mapping[int, str]
    ) -> tuple[object, object, object]:
        """The layer, the source id and the part of a feature that an error
        names, if any. An error of a whole feature names no part."""
        if reference is None:
            return None, None, None
        alias = aliases[number(reference['layer'])]
        fid = number(reference['fid'])
        part = reference['part']
        return (
            alias,
            source_id(self.ids, alias, fid),
            None if part is None else number(part),
        )

    def summary(self, entry: c.Mapping[str, object]) -> bool:
        warnings = entry.get('warnings', [])
        if not isinstance(warnings, list):
            return False
        self.warnings = [str(w) for w in t.cast('list[object]', warnings)]
        return True
