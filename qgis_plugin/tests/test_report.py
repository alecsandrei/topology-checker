"""The report of the executable, read line by line: the errors of each rule
with their geometry as WKB, the ids of the source features, and what is not
of the report left out."""

from __future__ import annotations

import json
import math
import struct
import types
import typing as t

import pytest
from qgis.core import QgsGeometry

from topology_rules.core import report
from topology_rules.core.checks import Job
from topology_rules.core.errors import Gathered, from_wkb
from topology_rules.core.report import Report

from .live import Recorder

DANGLES = 'line/must-not-have-dangles'
CROSSING = 'line/must-not-intersect-with'


def job(rule: str, layer: str, other: str | None = None) -> Job:
    """A job as the report needs it: the name of its rule and layers."""
    return t.cast(
        Job,
        types.SimpleNamespace(
            rule=types.SimpleNamespace(
                name=rule, short_name=rule, plain_name=rule
            ),
            layer=types.SimpleNamespace(name=layer),
            other=None if other is None else types.SimpleNamespace(name=other),
        ),
    )


JOBS = [job(DANGLES, 'roads'), job(CROSSING, 'rivers', 'roads')]
# The source features of the exported layers: the n-th came from the n-th id
IDS = {'roads': (10, 20, 30), 'rivers': (7,)}


def reading(jobs: list[Job] = JOBS) -> tuple[Report, Recorder]:
    feedback = Recorder()
    gathered = Gathered(jobs, {'roads': 'r-id', 'rivers': 'v-id'}, feedback)
    return Report(gathered, IDS), feedback


def point(x: float, y: float) -> dict[str, object]:
    return {'Point': {'x': x, 'y': y}}


def feature(layer: int, fid: object, part: object = 0) -> dict[str, object]:
    return {'layer': layer, 'fid': fid, 'part': part}


def error(
    geometry: object,
    origin: object,
    destination: object = None,
    rule: str = DANGLES,
    kind: str = 'dangle',
) -> dict[str, object]:
    return {
        'rule': rule,
        'error_type': kind,
        'geometry': geometry,
        'origin': origin,
        'destination': destination,
    }


def dangles(*errors: object) -> dict[str, object]:
    return {
        'spec': {'rule': DANGLES, 'layer': 2, 'other': None},
        'layer': 'roads',
        'other': None,
        'counts': {'dangle': len(errors)},
        'errors': list(errors),
    }


def crossings(*errors: object) -> dict[str, object]:
    return {
        'spec': {'rule': CROSSING, 'layer': 1, 'other': 2},
        'layer': 'rivers',
        'other': 'roads',
        'counts': {'intersection': len(errors)},
        'errors': list(errors),
    }


FIRST = dangles(
    error(point(0, 0), feature(2, 1)), error(point(5, 5), feature(2, 3))
)
SECOND = crossings(
    error(
        point(2.5, 0),
        feature(1, 1),
        # An error about a whole feature names no part
        feature(2, 2, None),
        rule=CROSSING,
        kind='intersection',
    )
)
LAST = {'tolerance': 1e-8, 'warnings': ['roads has 1 invalid geometry']}


def line(value: object) -> str:
    return json.dumps(value)


def values(feedback: Recorder) -> list[list[tuple[object, ...]]]:
    return [
        [found.values for found in done.errors] for done in feedback.checks()
    ]


EXPECTED = [
    [
        (0, DANGLES, 'dangle', 'roads', 10, 0, None, None, None),
        (1, DANGLES, 'dangle', 'roads', 30, 0, None, None, None),
    ],
    [(2, CROSSING, 'intersection', 'rivers', 7, 0, 'roads', 20, None)],
]


def test_each_rule_is_gathered_as_its_line_is_read():
    """In the order of the rules, numbered over all of them, with the ids
    of the source features and the layers by their names."""
    read, feedback = reading()
    assert read.line(line(FIRST))
    assert [done.index for done in feedback.checks()] == [0]
    assert not read.complete
    assert read.line(line(SECOND))
    assert not read.complete
    assert read.line(line(LAST))
    assert read.complete
    assert read.warnings == LAST['warnings']
    assert values(feedback) == EXPECTED
    assert [(d.count, d.total) for d in feedback.checks()] == [(2, 2), (1, 2)]
    assert read.gathered.counts == [2, 1]
    assert [
        found.geometry.asWkt() for found in feedback.checks()[0].errors
    ] == [
        'MultiPoint ((0 0))',
        'MultiPoint ((5 5))',
    ]


def test_an_older_tool_tells_all_rules_at_the_end():
    """Its one line has the rules, which are read as those of --stream."""
    read, feedback = reading()
    assert read.line(line({**LAST, 'rules': [FIRST, SECOND]}))
    assert read.complete
    assert read.warnings == LAST['warnings']
    assert values(feedback) == EXPECTED


def test_a_rule_past_the_last_is_not_read():
    read, feedback = reading()
    for value in (FIRST, SECOND):
        assert read.line(line(value))
    assert not read.line(line(FIRST))
    assert len(feedback.checks()) == 2


def test_a_feature_outside_its_export_keeps_its_number():
    assert report.source_id(IDS, 'roads', 0) == 0
    assert report.source_id(IDS, 'roads', 4) == 4
    assert report.source_id(IDS, 'roads', 3) == 30
    assert report.source_id(IDS, 'lakes', 2) == 2
    assert report.source_id(IDS, 'roads', None) is None


def other_rule() -> dict[str, object]:
    value = json.loads(line(FIRST))
    value['spec']['rule'] = 'line/no-such-rule'
    return value


def changed(path: tuple[object, ...], new: object) -> dict[str, object]:
    """The first rule with a value changed, such as that of one of the
    keys of its first error."""
    value = json.loads(line(FIRST))
    here = value
    for key in path[:-1]:
        here = here[key]
    here[path[-1]] = new
    return value


GARBAGE = {
    'empty': '',
    'text': 'reading roads',
    'null': 'null',
    'list': '[1, 2]',
    'string': '"rules"',
    'number': '12',
    'thousands of digits': '9' * 5000,
    'nested deep': '[' * 100000,
    'cut off': line(FIRST)[: len(line(FIRST)) // 2],
    'no spec': line({'spec': None, 'errors': [1]}),
    'another rule': line(other_rule()),
    'layer of digits': line(FIRST).replace(
        '"layer": 2', '"layer": ' + '9' * 5000
    ),
    'layer a bool': line(changed(('spec', 'layer'), True)),
    'counts a list': line(changed(('counts',), [2])),
    'count a float': line(changed(('counts', 'dangle'), 2.0)),
    'count a bool': line(changed(('counts', 'dangle'), True)),
    'errors a number': line(changed(('errors',), 5)),
    'no geometry': line(changed(('errors', 0, 'geometry'), None)),
    'unknown geometry': line(changed(('errors', 0, 'geometry'), {'Circle': 1})),
    'two geometries': line(
        changed(('errors', 0, 'geometry'), {**point(0, 0), 'Line': 1})
    ),
    'coordinate a text': line(
        changed(('errors', 0, 'geometry', 'Point', 'y'), 'a')
    ),
    'coordinate too large': line(
        changed(('errors', 0, 'geometry'), point(10**400, 0))
    ),
    'unknown layer': line(changed(('errors', 0, 'origin', 'layer'), 5)),
    'fid a bool': line(changed(('errors', 0, 'origin', 'fid'), False)),
    'fid a text': line(changed(('errors', 0, 'origin', 'fid'), '1')),
    'part a text': line(changed(('errors', 0, 'origin', 'part'), '0')),
    'origin a list': line(changed(('errors', 0, 'origin'), [2, 1, 0])),
    'warnings a text': line({'tolerance': 1e-8, 'warnings': 'many'}),
    'rules a text': line({'tolerance': 1e-8, 'rules': 'all'}),
}


@pytest.mark.parametrize('text', list(GARBAGE.values()), ids=list(GARBAGE))
def test_a_line_that_is_not_of_the_report_says_nothing(text: str):
    """It is left out, and the next line of the report is read as if it
    were not there."""
    read, feedback = reading()
    assert not read.line(text)
    assert feedback.checks() == []
    assert read.warnings is None
    assert read.line(line(FIRST))
    assert read.line(line(SECOND))
    assert values(feedback) == EXPECTED


def test_a_rule_is_not_gathered_once_the_run_is_canceled():
    read, feedback = reading()
    assert read.line(line(FIRST))
    feedback.cancel()
    assert not read.line(line(SECOND))
    assert len(feedback.checks()) == 1
    assert read.gathered.counts == [2]


# Geometries


SQUARE = [
    {'x': 0.0, 'y': 0.0},
    {'x': 4.0, 'y': 0.0},
    {'x': 4.0, 'y': 4.0},
    {'x': 0.0, 'y': 0.0},
]
HOLE = [
    {'x': 1.0, 'y': 1.0},
    {'x': 2.0, 'y': 1.0},
    {'x': 2.0, 'y': 2.0},
    {'x': 1.0, 'y': 1.0},
]
LINE = [{'x': 0.0, 'y': 0.0}, {'x': 1.5, 'y': -2.0}, {'x': 3.0, 'y': 1.0}]

GEOMETRIES: dict[str, tuple[object, str]] = {
    'Point': (point(1.5, -2), 'Point (1.5 -2)'),
    'Line': (
        {'Line': {'start': {'x': 0, 'y': 0}, 'end': {'x': 1, 'y': 1}}},
        'LineString (0 0, 1 1)',
    ),
    'LineString': ({'LineString': LINE}, 'LineString (0 0, 1.5 -2, 3 1)'),
    'Polygon': (
        {'Polygon': {'exterior': SQUARE, 'interiors': [HOLE]}},
        'Polygon ((0 0, 4 0, 4 4, 0 0),(1 1, 2 1, 2 2, 1 1))',
    ),
    'MultiPoint': (
        {'MultiPoint': [{'x': 0, 'y': 0}, {'x': 1, 'y': 2}]},
        'MultiPoint ((0 0),(1 2))',
    ),
    'MultiLineString': (
        {'MultiLineString': [LINE, LINE[:2]]},
        'MultiLineString ((0 0, 1.5 -2, 3 1),(0 0, 1.5 -2))',
    ),
    'MultiPolygon': (
        {
            'MultiPolygon': [
                {'exterior': SQUARE, 'interiors': [HOLE]},
                {'exterior': HOLE, 'interiors': []},
            ]
        },
        'MultiPolygon (((0 0, 4 0, 4 4, 0 0),(1 1, 2 1, 2 2, 1 1)),'
        '((1 1, 2 1, 2 2, 1 1)))',
    ),
    'GeometryCollection': (
        {
            'GeometryCollection': [
                point(1, 2),
                {'LineString': LINE},
                {'GeometryCollection': [point(3, 4)]},
            ]
        },
        'GeometryCollection (Point (1 2),LineString (0 0, 1.5 -2, 3 1),'
        'GeometryCollection (Point (3 4)))',
    ),
    'Rect': (
        {'Rect': {'min': {'x': 0, 'y': 1}, 'max': {'x': 2, 'y': 3}}},
        'Polygon ((2 1, 2 3, 0 3, 0 1, 2 1))',
    ),
    'Triangle': (
        {'Triangle': [{'x': 0, 'y': 0}, {'x': 1, 'y': 0}, {'x': 0, 'y': 1}]},
        'Polygon ((0 0, 1 0, 0 1, 0 0))',
    ),
}


@pytest.mark.parametrize('name', list(GEOMETRIES))
def test_a_geometry_is_the_wkb_of_its_shape(name: str):
    """As QGIS writes the shape, byte for byte."""
    geometry, wkt = GEOMETRIES[name]
    expected = QgsGeometry.fromWkt(wkt)
    assert not expected.isNull()
    made = report.wkb(json.loads(json.dumps(geometry)))
    assert made == bytes(expected.asWkb())
    assert from_wkb(made).asWkt() == expected.asWkt()


@pytest.mark.parametrize(
    'value',
    [0.1, 1 / 3, 5e-324, -1.7976931348623157e308, -0.0, 4.4e15 + 0.5],
)
def test_a_coordinate_is_kept_to_the_last_bit(value: float):
    text = line(point(value, value))
    made = report.wkb(json.loads(text))
    assert struct.unpack('<2d', made[5:]) == (value, value)
    vertex = from_wkb(made).vertexAt(0)
    assert vertex.x() == value
    assert math.copysign(1, vertex.y()) == math.copysign(1, value)
