"""A layer of 100000 lines through the dock: it finishes, it does not freeze
the interface, and the counts are right. Skip it with `-m "not slow"`."""

from __future__ import annotations

import time

import pytest
from qgis.core import QgsFeature, QgsGeometry, QgsProject, QgsVectorLayer
from qgis.PyQt.QtWidgets import QApplication

from topology_rules.panel.dock import Dock

from .helpers import add, queue_column, shown

pytestmark = pytest.mark.slow

DANGLES = 'line/must-not-have-dangles'
CHAIN = 99_000
SPIKES = 1_000
# The free ends of the chain, and both ends of each separate line
ERRORS = 2 + 2 * SPIKES
# The longest the interface may stand still, and the whole run
MAX_STALL = 3.0
MAX_RUN = 180.0


def big_layer() -> QgsVectorLayer:
    """A chain of connected lines, and lines far from each other."""
    layer = QgsVectorLayer('LineString?crs=EPSG:32633', 'big', 'memory')
    features: list[QgsFeature] = []
    for index in range(CHAIN):
        feature = QgsFeature()
        feature.setGeometry(
            QgsGeometry.fromWkt(f'LINESTRING({index} 0, {index + 1} 0)')
        )
        features.append(feature)
    for index in range(SPIKES):
        feature = QgsFeature()
        feature.setGeometry(
            QgsGeometry.fromWkt(
                f'LINESTRING({3 * index} 100, {3 * index + 1} 100)'
            )
        )
        features.append(feature)
    assert layer.dataProvider().addFeatures(features)[0]
    layer.updateExtents()
    assert layer.featureCount() == CHAIN + SPIKES
    QgsProject.instance().addMapLayer(layer)
    return layer


def run_watching_the_interface(dock: Dock) -> float:
    """Runs the dock and returns the longest time that the event loop was
    kept busy by a single step, which is what a user feels as a freeze."""
    dock.run_button.click()
    started = time.monotonic()
    longest = 0.0
    while dock.task is not None:
        assert time.monotonic() - started < MAX_RUN, 'the run did not end'
        before = time.monotonic()
        QApplication.processEvents()
        longest = max(longest, time.monotonic() - before)
        time.sleep(0.01)
    return longest


def test_a_big_layer_is_checked_without_freezing_the_interface(dock: Dock):
    # One run, as it takes a while: the counts, the results and the stall
    add(dock, DANGLES, big_layer())
    stall = run_watching_the_interface(dock)
    assert queue_column(dock, 3) == [str(ERRORS)]
    assert shown(dock) == ERRORS
    assert stall < MAX_STALL
