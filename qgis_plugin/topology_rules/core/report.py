"""The report of the executable, read as it comes.

With `--json validate --stream` the executable says the report of each rule
on a line of its own as soon as the rule is done, in the order of the rules,
then a line with the tolerance and the warnings. An older one says the
whole report on one line at the end, whose `rules` are the same reports.
Each error has its geometry in the form of geo-types, which is made WKB
here.

Each report names its rule and layers, which have to be those of the next
rule of the run: a run whose report is of other rules, or of more, fails.
So does a run with a line of the report that cannot be read: no line after
it is read.

Each error is made small as soon as it is parsed, so that a rule of many
errors takes a fraction of the memory of its parsed line.
"""

from __future__ import annotations

import collections.abc as c
import json
import re
import struct
import sys
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
MALFORMED = (
    KeyError,
    TypeError,
    ValueError,
    AttributeError,
    IndexError,
    OverflowError,
    RecursionError,
    struct.error,
)

# The start of a line of the report, which a line that is not of it does
# not have: the report of a rule, or the last line
REPORT_LINE = re.compile(r'\s*\{\s*"(?:spec|tolerance)"')
# How much of a line of the report that cannot be read the log shows
SHOWN_CHARS = 200

Coord = t.Mapping[str, float]
# A feature that an error names: the number of its layer, its id and part
Reference = tuple[int, int, 'int | None']


class Parsed(t.NamedTuple):
    """An error of the report as it is parsed: the strings of its rule and
    kind are shared with the other errors, and its geometry is WKB."""

    rule: str
    error_type: str
    origin: Reference | None
    destination: Reference | None
    wkb: bytes


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


def put_geometry(geometry: object, out: bytearray) -> None:
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
            put_geometry(member, out)
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
    put_geometry(geometry, out)
    return bytes(out)


def number(value: object) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(value)
    return value


def text(value: object) -> str:
    if not isinstance(value, str):
        raise TypeError(value)
    return value


def reference(value: t.Any) -> Reference | None:
    """The feature that an error names, if any. An error of a whole feature
    names no part."""
    if value is None:
        return None
    part = value['part']
    return (
        number(value['layer']),
        number(value['fid']),
        None if part is None else number(part),
    )


def parsed(entry: dict[str, t.Any]) -> object:
    """An object of a line as it is parsed: an error is made a Parsed,
    anything else stays as it is."""
    if 'error_type' not in entry or 'geometry' not in entry:
        return entry
    return Parsed(
        sys.intern(str(entry['rule'])),
        sys.intern(str(entry['error_type'])),
        reference(entry['origin']),
        reference(entry['destination']),
        wkb(entry['geometry']),
    )


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
        # Whether a line of the report could not be read
        self.broken = False

    @property
    def complete(self) -> bool:
        """Whether every rule and the warnings were read."""
        return (
            len(self.gathered.counts) == len(self.gathered.jobs)
            and self.warnings is not None
        )

    @property
    def over(self) -> bool:
        """Whether no more of the report is read: it is complete, or a line
        of it could not be read."""
        return self.complete or self.broken

    def line(self, line: str) -> bool:
        """Reads a line of the output, and says whether it was read as a
        line of the report. One that is not of the report, such as a message
        of the tool, says nothing of the run, nor does any line once the
        report is over. One of the report that cannot be read breaks it."""
        if self.over:
            return False
        try:
            value = json.loads(line, object_hook=parsed)
        except MALFORMED as error:
            if REPORT_LINE.match(line):
                self.unreadable(line, error)
            return False
        if not isinstance(value, dict):
            return False
        entry = t.cast('dict[str, object]', value)
        if 'spec' in entry:
            return self.rule(entry, line)
        if 'tolerance' not in entry:
            return False
        # An older executable has the rules in the report, which are all
        # done: the stage does not go through them
        rules = entry.get('rules', [])
        warnings = entry.get('warnings', [])
        if not isinstance(rules, list) or not isinstance(warnings, list):
            self.unreadable(line, TypeError())
            return False
        rules = t.cast('list[object]', rules)
        for position, rule in enumerate(rules):
            if not self.rule(rule, line, position < len(rules) - 1):
                return False
        self.warnings = [str(w) for w in t.cast('list[object]', warnings)]
        return True

    def rule(self, value: object, line: str, more: bool = False) -> bool:
        """Reads the report of the next rule, which is gathered once all of
        it is read, with more saying that the rule after it is done too.
        One that cannot be read breaks the report. One of another rule or
        other layers than the next rule, or past the last, fails the run,
        which keeps the rules before it.

        A run that is canceled stops at once, without the rule."""
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
            errors = entry['errors']
            if not isinstance(errors, list):
                raise TypeError(errors)
            errors = t.cast('list[object]', errors)
            # Each parsed error goes as soon as it is a row, from the first
            errors.reverse()
            found: list[Found] = []
            while errors:
                if self.gathered.feedback.isCanceled():
                    return False
                error = errors.pop()
                if not isinstance(error, Parsed):
                    raise TypeError(error)
                values = (
                    violation,
                    error.rule,
                    error.error_type,
                    *self.feature(error.origin, aliases),
                    *self.feature(error.destination, aliases),
                )
                found += found_in(from_wkb(error.wkb), values)
                violation += 1
        except MALFORMED as error:
            self.unreadable(line, error)
            return False
        self.violation = violation
        self.numbers = numbers
        self.gathered.add(found, count, more)
        return True

    def unreadable(self, line: str, error: BaseException) -> None:
        """Tells the log of a line of the report that cannot be read, after
        which no line is read."""
        self.broken = True
        start = line.strip()
        if len(start) > SHOWN_CHARS:
            start = start[:SHOWN_CHARS] + '…'
        self.gathered.feedback.pushInfo(
            tr(
                'The report of rule {number} cannot be read ({error}): {line}'
            ).format(
                number=len(self.gathered.counts) + 1,
                error=type(error).__name__,
                line=start,
            )
        )

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
            tr(
                "The executable's report does not match the rules that "
                'were run.'
            )
        )

    def feature(
        self, reference: Reference | None, aliases: c.Mapping[int, str]
    ) -> tuple[object, object, object]:
        """The layer, the source id and the part of a feature that an error
        names, if any."""
        if reference is None:
            return None, None, None
        layer, fid, part = reference
        alias = aliases[layer]
        return alias, source_id(self.ids, alias, fid), part
