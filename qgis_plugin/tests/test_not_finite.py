"""Unit tests for not_finite() geometry coordinate verification."""

from __future__ import annotations

import math

import pytest
from qgis.core import (
    QgsGeometry,
    QgsPointXY,
)

from topology_rules.core.inputs import invalid_reason, not_finite

NON_FINITE_VALUES = [math.nan, math.inf, -math.inf]


# ---------------------------------------------------------------------------
# Valid geometries -> False
# ---------------------------------------------------------------------------


def test_valid_point_is_finite():
    pt = QgsGeometry.fromPointXY(QgsPointXY(10.5, 20.3))
    assert not_finite(pt) is False
    assert invalid_reason(pt) is None


def test_valid_linestring_is_finite():
    line = QgsGeometry.fromPolylineXY(
        [QgsPointXY(0.0, 0.0), QgsPointXY(10.0, 5.0), QgsPointXY(20.0, 10.0)]
    )
    assert not_finite(line) is False
    assert invalid_reason(line) is None


def test_valid_multilinestring_is_finite():
    mline = QgsGeometry.fromMultiPolylineXY(
        [
            [QgsPointXY(0.0, 0.0), QgsPointXY(5.0, 5.0)],
            [QgsPointXY(10.0, 10.0), QgsPointXY(15.0, 15.0)],
        ]
    )
    assert not_finite(mline) is False
    assert invalid_reason(mline) is None


def test_valid_multipoint_is_finite():
    mp = QgsGeometry.fromMultiPointXY(
        [QgsPointXY(1.0, 2.0), QgsPointXY(3.0, 4.0), QgsPointXY(5.0, 6.0)]
    )
    assert not_finite(mp) is False
    assert invalid_reason(mp) is None


def test_valid_polygon_is_finite():
    poly = QgsGeometry.fromPolygonXY(
        [
            [
                QgsPointXY(0.0, 0.0),
                QgsPointXY(10.0, 0.0),
                QgsPointXY(10.0, 10.0),
                QgsPointXY(0.0, 10.0),
                QgsPointXY(0.0, 0.0),
            ]
        ]
    )
    assert not_finite(poly) is False


def test_valid_multipolygon_is_finite():
    mpoly = QgsGeometry.fromMultiPolygonXY(
        [
            [
                [
                    QgsPointXY(0.0, 0.0),
                    QgsPointXY(10.0, 0.0),
                    QgsPointXY(10.0, 10.0),
                    QgsPointXY(0.0, 10.0),
                    QgsPointXY(0.0, 0.0),
                ]
            ],
            [
                [
                    QgsPointXY(20.0, 20.0),
                    QgsPointXY(30.0, 20.0),
                    QgsPointXY(30.0, 30.0),
                    QgsPointXY(20.0, 30.0),
                    QgsPointXY(20.0, 20.0),
                ]
            ],
        ]
    )
    assert not_finite(mpoly) is False


# ---------------------------------------------------------------------------
# Empty & null geometries -> False
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    'wkt',
    [
        'POINT EMPTY',
        'LINESTRING EMPTY',
        'POLYGON EMPTY',
        'MULTIPOINT EMPTY',
        'MULTILINESTRING EMPTY',
        'MULTIPOLYGON EMPTY',
    ],
)
def test_empty_geometries_are_not_rejected_as_non_finite(wkt: str):
    geom = QgsGeometry.fromWkt(wkt)
    assert not_finite(geom) is False


def test_null_geometry_is_not_rejected_as_non_finite():
    null_geom = QgsGeometry()
    assert not_finite(null_geom) is False
    assert invalid_reason(null_geom) is None


# ---------------------------------------------------------------------------
# Geometries with NaN / Inf -> True
# ---------------------------------------------------------------------------


@pytest.mark.parametrize('value', NON_FINITE_VALUES)
def test_point_with_non_finite_coordinate(value: float):
    pt_x = QgsGeometry.fromPointXY(QgsPointXY(value, 5.0))
    assert not_finite(pt_x) is True
    assert invalid_reason(pt_x) == 'coordinates must be finite'

    pt_y = QgsGeometry.fromPointXY(QgsPointXY(5.0, value))
    assert not_finite(pt_y) is True
    assert invalid_reason(pt_y) == 'coordinates must be finite'


@pytest.mark.parametrize('value', NON_FINITE_VALUES)
def test_linestring_with_non_finite_coordinate(value: float):
    # Non-finite in first vertex
    line_start = QgsGeometry.fromPolylineXY(
        [QgsPointXY(value, 0.0), QgsPointXY(10.0, 10.0)]
    )
    assert not_finite(line_start) is True
    assert invalid_reason(line_start) == 'coordinates must be finite'

    # Non-finite in middle vertex
    line_mid = QgsGeometry.fromPolylineXY(
        [QgsPointXY(0.0, 0.0), QgsPointXY(5.0, value), QgsPointXY(10.0, 10.0)]
    )
    assert not_finite(line_mid) is True
    assert invalid_reason(line_mid) == 'coordinates must be finite'

    # Non-finite in end vertex
    line_end = QgsGeometry.fromPolylineXY(
        [QgsPointXY(0.0, 0.0), QgsPointXY(10.0, value)]
    )
    assert not_finite(line_end) is True
    assert invalid_reason(line_end) == 'coordinates must be finite'


@pytest.mark.parametrize('value', NON_FINITE_VALUES)
def test_multilinestring_with_non_finite_coordinate(value: float):
    # Non-finite in first part
    mline1 = QgsGeometry.fromMultiPolylineXY(
        [
            [QgsPointXY(0.0, 0.0), QgsPointXY(value, 5.0)],
            [QgsPointXY(10.0, 10.0), QgsPointXY(15.0, 15.0)],
        ]
    )
    assert not_finite(mline1) is True
    assert invalid_reason(mline1) == 'coordinates must be finite'

    # Non-finite in second part
    mline2 = QgsGeometry.fromMultiPolylineXY(
        [
            [QgsPointXY(0.0, 0.0), QgsPointXY(5.0, 5.0)],
            [QgsPointXY(10.0, 10.0), QgsPointXY(15.0, value)],
        ]
    )
    assert not_finite(mline2) is True
    assert invalid_reason(mline2) == 'coordinates must be finite'


@pytest.mark.parametrize('value', NON_FINITE_VALUES)
def test_multipoint_with_secondary_vertex_non_finite(value: float):
    # Bounding box is finite from vertex 1, but vertex 2 has NaN/Inf
    mp_second = QgsGeometry.fromMultiPointXY(
        [QgsPointXY(1.0, 1.0), QgsPointXY(2.0, value)]
    )
    assert not_finite(mp_second) is True
    assert invalid_reason(mp_second) == 'coordinates must be finite'

    # Third vertex non-finite
    mp_third = QgsGeometry.fromMultiPointXY(
        [QgsPointXY(1.0, 1.0), QgsPointXY(2.0, 2.0), QgsPointXY(value, 3.0)]
    )
    assert not_finite(mp_third) is True
    assert invalid_reason(mp_third) == 'coordinates must be finite'


@pytest.mark.parametrize('value', NON_FINITE_VALUES)
def test_multipoint_with_primary_vertex_non_finite(value: float):
    mp_first = QgsGeometry.fromMultiPointXY(
        [QgsPointXY(value, 1.0), QgsPointXY(2.0, 2.0)]
    )
    assert not_finite(mp_first) is True
    assert invalid_reason(mp_first) == 'coordinates must be finite'


@pytest.mark.parametrize('value', NON_FINITE_VALUES)
def test_polygon_with_non_finite_coordinate(value: float):
    poly = QgsGeometry.fromPolygonXY(
        [
            [
                QgsPointXY(0.0, 0.0),
                QgsPointXY(value, 0.0),
                QgsPointXY(10.0, 10.0),
                QgsPointXY(0.0, 10.0),
                QgsPointXY(0.0, 0.0),
            ]
        ]
    )
    assert not_finite(poly) is True


@pytest.mark.parametrize('value', NON_FINITE_VALUES)
def test_multipolygon_with_non_finite_coordinate(value: float):
    mpoly = QgsGeometry.fromMultiPolygonXY(
        [
            [
                [
                    QgsPointXY(0.0, 0.0),
                    QgsPointXY(10.0, 0.0),
                    QgsPointXY(10.0, 10.0),
                    QgsPointXY(0.0, 10.0),
                    QgsPointXY(0.0, 0.0),
                ]
            ],
            [
                [
                    QgsPointXY(20.0, 20.0),
                    QgsPointXY(value, 20.0),
                    QgsPointXY(30.0, 30.0),
                    QgsPointXY(20.0, 30.0),
                    QgsPointXY(20.0, 20.0),
                ]
            ],
        ]
    )
    assert not_finite(mpoly) is True
