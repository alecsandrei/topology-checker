"""The files that a run makes in its folder of scratch files are let go of
before the folder is removed: Windows cannot remove a file that is open."""

from __future__ import annotations

import collections.abc as c
import contextlib
import tempfile
import weakref
from pathlib import Path

import pytest
from qgis.core import QgsProject, QgsVectorLayer
from qgis.PyQt import sip

from topology_rules.algorithms import utilities
from topology_rules.provider import Provider

from .helpers import line_layer, run_algorithm

CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


@pytest.fixture(autouse=True)
def ready(executable: str, provider: Provider) -> None:
    """The real executable and the provider, as in a QGIS session."""


@pytest.fixture
def leaks(monkeypatch: pytest.MonkeyPatch) -> list[str]:
    """What is still open when the folder of scratch files is removed: the
    layers that the algorithm made, and has not let go of."""
    found: list[str] = []
    made: list[weakref.ref[QgsVectorLayer]] = []

    class Watched(QgsVectorLayer):
        def __init__(self, *args: object):
            super().__init__(*args)  # type: ignore[arg-type]
            made.append(weakref.ref(self))

    @contextlib.contextmanager
    def watched() -> c.Iterator[str]:
        with tempfile.TemporaryDirectory(prefix='topology-checker-') as folder:
            try:
                yield folder
            finally:
                found.extend(
                    layer.source()
                    for layer in (ref() for ref in made)
                    if layer is not None
                    and not sip.isdeleted(layer)
                    and folder in layer.source()
                )

    monkeypatch.setattr(utilities, 'scratch', watched)
    monkeypatch.setattr(utilities, 'QgsVectorLayer', Watched)
    return found


@pytest.mark.parametrize('name', ['merge_lines', 'explode_lines'])
def test_the_output_of_a_utility_is_closed_before_its_folder_goes(
    tmp_path: Path, name: str, leaks: list[str]
):
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    ran = run_algorithm(
        name, {'INPUT': layer.id(), 'OUTPUT': str(tmp_path / 'out.gpkg')}
    )
    assert ran.ok, ran.log
    assert leaks == []


@pytest.mark.parametrize('name', ['merge_lines', 'explode_lines'])
def test_the_output_of_a_utility_that_fails_is_closed_before_its_folder_goes(
    tmp_path: Path, name: str, leaks: list[str]
):
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    # In a folder that is not there, so that the sink cannot be made
    ran = run_algorithm(
        name,
        {'INPUT': layer.id(), 'OUTPUT': str(tmp_path / 'none' / 'out.gpkg')},
    )
    assert not ran.ok
    assert leaks == []
