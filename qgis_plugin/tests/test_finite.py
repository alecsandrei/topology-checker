"""Coordinates that are not finite, which the tool rejects: they are found in
the features before it runs, and finite ones are not taken for them."""

from __future__ import annotations

import collections.abc as c
import math

import pytest
from qgis.core import (
    QgsCircularString,
    QgsGeometry,
    QgsMultiPoint,
    QgsPoint,
    QgsPointXY,
)

from topology_rules.core.inputs import invalid_reason, not_finite

NOT_FINITE = 'coordinates must be finite'
SQUARE = [(0, 0), (10, 0), (10, 10), (0, 10), (0, 0)]

Points = list[tuple[float, float]]


def xy(points: Points) -> list[QgsPointXY]:
    return [QgsPointXY(x, y) for x, y in points]


def point(x: float, y: float) -> QgsGeometry:
    return QgsGeometry.fromPointXY(QgsPointXY(x, y))


def line(*points: tuple[float, float]) -> QgsGeometry:
    return QgsGeometry.fromPolylineXY(xy(list(points)))


def lines(*parts: Points) -> QgsGeometry:
    return QgsGeometry.fromMultiPolylineXY([xy(part) for part in parts])


def points(*members: tuple[float, float]) -> QgsGeometry:
    return QgsGeometry.fromMultiPointXY(xy(list(members)))


def polygons(*rings: Points) -> QgsGeometry:
    return QgsGeometry.fromMultiPolygonXY([[xy(ring)] for ring in rings])


def arc(x: float) -> QgsGeometry:
    return QgsGeometry(
        QgsCircularString(QgsPoint(0, 0), QgsPoint(x, 1), QgsPoint(2, 0))
    )


def with_an_empty_point() -> QgsGeometry:
    members = QgsMultiPoint()
    members.addGeometry(QgsPoint(1, 1))
    members.addGeometry(QgsPoint())
    return QgsGeometry(members)


FINITE: dict[str, c.Callable[[], QgsGeometry]] = {
    'point': lambda: point(10.5, 20.3),
    'line': lambda: line((0, 0), (10, 5), (20, 10)),
    'lines': lambda: lines([(0, 0), (5, 5)], [(10, 10), (15, 15)]),
    'points': lambda: points((1, 2), (3, 4)),
    'polygons': lambda: polygons(SQUARE, [(x + 20, y) for x, y in SQUARE]),
    'arc': lambda: arc(1),
    # Its length is infinite, its coordinates are not
    'huge line': lambda: line((-1e308, 0), (1e308, 0)),
    'huge lines': lambda: lines([(0, 0), (1, 0)], [(-1e308, 5), (1e308, 5)]),
    'points with an empty one': with_an_empty_point,
    'null': QgsGeometry,
}
EMPTY = [
    'POINT EMPTY',
    'LINESTRING EMPTY',
    'POLYGON EMPTY',
    'CURVEPOLYGON EMPTY',
    'MULTIPOINT EMPTY',
    'MULTILINESTRING EMPTY',
    'MULTIPOLYGON EMPTY',
]
# Points and lines whose coordinate at some place is the value
POINTS_AND_LINES: dict[str, c.Callable[[float], QgsGeometry]] = {
    'point x': lambda v: point(v, 5),
    'point y': lambda v: point(5, v),
    'line start': lambda v: line((v, 0), (10, 10)),
    'line middle': lambda v: line((0, 0), (5, v), (10, 10)),
    'line end': lambda v: line((0, 0), (10, v)),
    'lines first': lambda v: lines([(0, 0), (v, 5)], [(10, 10), (15, 15)]),
    'lines second': lambda v: lines([(0, 0), (5, 5)], [(10, 10), (15, v)]),
    'points first': lambda v: points((v, 1), (2, 2)),
    'points last': lambda v: points((1, 1), (2, 2), (v, 3)),
    'arc': arc,
}
POLYGONS: dict[str, c.Callable[[float], QgsGeometry]] = {
    'polygon': lambda v: polygons([(0, 0), (v, 0), (10, 10), (0, 0)]),
    'polygons second': lambda v: polygons(
        SQUARE, [(20, 20), (v, 20), (30, 30), (20, 20)]
    ),
}
VALUES = pytest.mark.parametrize(
    'value', [math.nan, math.inf, -math.inf], ids=['nan', 'inf', '-inf']
)


@pytest.mark.parametrize('name', FINITE)
def test_finite_coordinates(name: str):
    geometry = FINITE[name]()
    assert not not_finite(geometry)
    assert invalid_reason(geometry) is None


@pytest.mark.parametrize('wkt', EMPTY)
def test_an_empty_geometry_has_no_coordinates_to_reject(wkt: str):
    geometry = QgsGeometry.fromWkt(wkt)
    assert geometry.isEmpty()
    assert not not_finite(geometry)
    assert invalid_reason(geometry) is None


@VALUES
@pytest.mark.parametrize('name', POINTS_AND_LINES)
def test_a_point_or_a_line_that_is_not_finite_is_rejected(
    name: str, value: float
):
    geometry = POINTS_AND_LINES[name](value)
    assert not_finite(geometry)
    assert invalid_reason(geometry) == NOT_FINITE


@VALUES
@pytest.mark.parametrize('name', POLYGONS)
def test_a_polygon_that_is_not_finite_is_found(name: str, value: float):
    """Its geometry is left out of the layer of errors."""
    assert not_finite(POLYGONS[name](value))
