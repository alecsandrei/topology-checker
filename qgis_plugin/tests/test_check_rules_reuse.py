"""Running the rules again into a GeoPackage that an earlier run wrote: the
file holds the errors of the last run and nothing else."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from qgis.core import QgsProject, QgsVectorLayer

from topology_rules.algorithms.check import COUNTS
from topology_rules.algorithms.rule import COUNT
from topology_rules.core.engine import Engine
from topology_rules.provider import Provider

from .helpers import line_layer, run_algorithm

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
# A ring has no ends, so it has no dangles
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)
LAYERS = ('errors_point', 'errors_line', 'errors_polygon')


@pytest.fixture(autouse=True)
def ready(run_engine: Engine, executable: str, provider: Provider) -> None:
    """The real executable and the provider, as in a QGIS session, and
    each of the two engines in turn."""


def in_project(*wkts: str, name: str = 'lines') -> QgsVectorLayer:
    layer = line_layer(*wkts, name=name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def check(layer: QgsVectorLayer, output: Path) -> dict[str, object]:
    ran = run_algorithm(
        'check_rules',
        {
            'RULES': json.dumps(
                [{'rule': DANGLES, 'layer': layer.id(), 'other': None}]
            ),
            'OUTPUT': str(output),
        },
    )
    assert ran.ok, ran.log
    return ran.results


def stored(output: Path) -> list[QgsVectorLayer]:
    """The layers of errors that the file has."""
    found = [
        QgsVectorLayer(f'{output}|layername={name}', name, 'ogr')
        for name in LAYERS
    ]
    return [layer for layer in found if layer.isValid()]


def errors_in(output: Path) -> int:
    return sum(layer.featureCount() for layer in stored(output))


def test_a_run_without_errors_leaves_no_errors_of_an_earlier_run(
    tmp_path: Path,
):
    output = tmp_path / 'errors.gpkg'
    assert check(in_project(*CHAIN), output)[COUNT] == 2
    check(in_project(*RING, name='ring'), output)
    assert errors_in(output) == 0


def test_the_count_of_a_run_without_errors_is_zero_after_an_earlier_run(
    tmp_path: Path,
):
    output = tmp_path / 'errors.gpkg'
    check(in_project(*CHAIN), output)
    results = check(in_project(*RING, name='ring'), output)
    assert results[COUNT] == 0
    assert json.loads(str(results[COUNTS])) == [0]


def test_the_count_matches_the_errors_in_the_file_when_a_run_is_repeated(
    tmp_path: Path,
):
    output = tmp_path / 'errors.gpkg'
    layer = in_project(*CHAIN)
    check(layer, output)
    results = check(layer, output)
    assert results[COUNT] == errors_in(output)


def test_the_ids_of_the_features_are_the_same_when_a_run_is_repeated(
    tmp_path: Path,
):
    # The ids of the layer have a hole, so that the features of the export
    # have other numbers than the features of the layer
    layer = in_project(*CHAIN, 'LINESTRING(0 5, 10 5)', name='holes')
    assert layer.dataProvider().deleteFeatures([2])
    output = tmp_path / 'errors.gpkg'

    def fids() -> list[int]:
        return sorted(
            int(feature['origin_fid'])
            for errors in stored(output)
            for feature in errors.getFeatures()
        )

    check(layer, output)
    first = fids()
    check(layer, output)
    assert fids() == first
    assert set(first) == {1, 3}
