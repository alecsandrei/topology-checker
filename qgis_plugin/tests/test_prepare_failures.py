"""Whatever goes wrong while a run is being prepared on the main thread, the
user is told: in the panel, and in the log of Processing. A run that is
dropped without a word looks like a button that does nothing."""

from __future__ import annotations

import json
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsCoordinateReferenceSystem, QgsProject
from qgis.gui import QgisInterface

from topology_rules.algorithms import check
from topology_rules.core import binary
from topology_rules.panel.dock import Dock
from topology_rules.utils import QgsCsException

from .helpers import (
    add,
    message_text,
    saved_layer,
    start_algorithm,
    wait_for,
)

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
TRANSFORM = 'the area cannot be transformed'
BOOM = 'the rules are not there'
KINDS = {'transform': TRANSFORM, 'rules': BOOM}


class NoTransform:
    """A transformation of coordinates that always fails."""

    def __init__(self, *args: object):
        pass

    def transformBoundingBox(self, *args: object) -> None:
        raise QgsCsException(TRANSFORM)


@pytest.fixture
def failing(
    request: pytest.FixtureRequest, monkeypatch: pytest.MonkeyPatch
) -> str:
    """What fails in the preparation: the transformation of the visible
    extent to the CRS of a layer, or the list of the rules of the tool."""
    kind: str = request.param
    if kind == 'transform':
        monkeypatch.setattr(check, 'QgsCoordinateTransform', NoTransform)
    else:

        def rules() -> t.NoReturn:
            raise RuntimeError(BOOM)

        monkeypatch.setattr(binary, 'rules', rules)
    return kind


def run_and_wait(dock: Dock) -> None:
    """Clicks Run, and waits for the panel to be free again. A run that was
    dropped without a word keeps the panel busy, which is the failure."""
    dock.run_button.click()
    try:
        wait_for(lambda: dock.task is None, 10)
    finally:
        # The panel is not left busy for the tests that follow
        dock.task = None


def layer_in_project(tmp_path: Path) -> str:
    layer = saved_layer(tmp_path, CHAIN, 'roads')
    QgsProject.instance().addMapLayer(layer)
    return str(layer.id())


@pytest.mark.parametrize('failing', KINDS, indirect=True)
def test_a_failure_while_the_run_is_prepared_is_a_message_in_the_panel(
    dock: Dock,
    qgis_iface: QgisInterface,
    tmp_path: Path,
    bar: t.Any,
    failing: str,
):
    layer = saved_layer(tmp_path, CHAIN, 'roads')
    QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, layer)
    if failing == 'transform':
        canvas = qgis_iface.mapCanvas()
        canvas.setDestinationCrs(QgsCoordinateReferenceSystem('EPSG:4326'))
        dock.extent_only.setChecked(True)
    run_and_wait(dock)
    assert KINDS[failing] in message_text(bar)


@pytest.mark.parametrize('failing', KINDS, indirect=True)
def test_the_panel_is_ready_again_after_a_failure_in_the_preparation(
    dock: Dock,
    qgis_iface: QgisInterface,
    tmp_path: Path,
    failing: str,
):
    layer = saved_layer(tmp_path, CHAIN, 'roads')
    QgsProject.instance().addMapLayer(layer)
    add(dock, DANGLES, layer)
    if failing == 'transform':
        canvas = qgis_iface.mapCanvas()
        canvas.setDestinationCrs(QgsCoordinateReferenceSystem('EPSG:4326'))
        dock.extent_only.setChecked(True)
    run_and_wait(dock)
    assert dock.task is None
    assert dock.rules_page.isEnabled()
    assert dock.run_button.text() != 'Cancel'


@pytest.mark.parametrize('failing', KINDS, indirect=True)
def test_a_failure_while_the_run_is_prepared_is_in_the_log_of_processing(
    executable: str, provider: object, tmp_path: Path, failing: str
):
    rules = [
        {'rule': DANGLES, 'layer': layer_in_project(tmp_path), 'other': None}
    ]
    scope = {
        'extent': {
            'rect': [0.0, 0.0, 1.0, 1.0],
            'crs': QgsCoordinateReferenceSystem('EPSG:4326').toWkt(),
        }
    }
    started = start_algorithm(
        'check_rules',
        {
            'RULES': json.dumps(rules),
            'SCOPE': json.dumps(scope),
            'OUTPUT': str(tmp_path / 'errors.gpkg'),
        },
    )
    wait_for(lambda: started.done, 10)
    ran = started.ran()
    assert not ran.ok
    assert KINDS[failing] in ran.log
