"""What a run does in a thread of its own, as Processing runs it.

When a QGIS object that Python wrapped is deleted in another thread, PyQt
tells the main thread later. The main thread then marks as deleted whatever
object Python has at that address. If QGIS has made another object there by
then, Python can no longer use that one. So a run must not leave such news.
The tests have QGIS make new objects in the thread after each run, where
they take the place of what the run deleted. Then the main thread gets
its news.
"""

from __future__ import annotations

import collections.abc as c
import functools
import shutil
import typing as t
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest
from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsFeature,
    QgsGeometry,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
    QgsProviderRegistry,
    QgsVectorDataProvider,
    QgsVectorFileWriter,
    QgsVectorLayer,
)
from qgis.PyQt import sip
from qgis.PyQt.QtCore import QCoreApplication

from topology_rules.core import package
from topology_rules.core.errors import Errors, write_errors
from topology_rules.core.inputs import chosen, prepare
from topology_rules.core.invalid import Invalid, append_invalid, error_fields
from topology_rules.core.layers import Input
from topology_rules.core.progress import Found

from .helpers import saved_layer
from .test_sources import CHAIN, FAR, fail_to_open, rewrites_open_files

T = t.TypeVar('T')
# How many runs a test does, and how many providers QGIS makes after each
# run. One of them often takes the place of an object that the run deleted,
# but not always
ROUNDS = 10
PROBES = 20


class Worker:
    """A thread that does one run after another, as a thread of Processing
    does, and makes providers of a source after each run."""

    def __init__(self) -> None:
        self.pool = ThreadPoolExecutor(1)
        self.source = ''
        self.providers: list[QgsVectorDataProvider] = []

    def after(self, work: c.Callable[[], T]) -> T:
        """What the work gives, done in the thread, which then makes the
        providers."""

        def run() -> T:
            result = work()
            registry = QgsProviderRegistry.instance()
            self.providers += [
                registry.createProvider('ogr', self.source)
                for _ in range(PROBES)
            ]
            return result

        return self.pool.submit(run).result()

    def deleted(self) -> int:
        """How many of the providers Python takes for deleted, once the main
        thread has heard what was deleted in the thread."""
        QCoreApplication.processEvents()
        return sum(sip.isdeleted(provider) for provider in self.providers)

    def close(self) -> None:
        """Deletes the providers in the thread, then ends it.

        Python leaves an object that it frees in another thread for the
        thread of the object to delete. A thread made in Python does that
        as it ends, when QGIS can no longer delete a provider.
        """
        self.pool.submit(self.providers.clear).result()
        self.pool.shutdown()
        QCoreApplication.processEvents()


@pytest.fixture
def worker() -> c.Iterator[Worker]:
    worker = Worker()
    yield worker
    worker.close()


def project_input(tmp_path: Path, worker: Worker) -> Input:
    """The input of a layer of the project in a GeoPackage, whose providers
    the worker makes."""
    layer = saved_layer(tmp_path, (*CHAIN, FAR))
    QgsProject.instance().addMapLayer(layer)
    worker.source = layer.source()
    return Input(layer.name(), layer, layer)


def test_preparing_an_input(tmp_path: Path, worker: Worker):
    item = project_input(tmp_path, worker)
    folder = tmp_path / 'run'
    folder.mkdir()
    for _ in range(ROUNDS):
        found = worker.after(
            lambda: prepare(
                item,
                folder,
                None,
                QgsProcessingContext(),
                QgsProcessingFeedback(),
            )
        )
        assert found.dataset.layer == 'lines'
    assert worker.deleted() == 0


def test_reading_an_input_for_the_package(tmp_path: Path, worker: Worker):
    item = project_input(tmp_path, worker)
    for _ in range(ROUNDS):
        found = worker.after(
            lambda: package.read(
                item,
                None,
                True,
                {},
                QgsProcessingContext(),
                QgsProcessingFeedback(),
            )
        )
        assert found.fids == [1, 2, 3]
    assert worker.deleted() == 0


@rewrites_open_files
def test_reading_again_an_input_that_qgis_failed_to_open(
    tmp_path: Path, worker: Worker
):
    item = project_input(tmp_path, worker)
    for _ in range(ROUNDS):
        fail_to_open(t.cast(QgsVectorLayer, item.layer))
        ids = worker.after(
            lambda: [
                feature.id()
                for feature in chosen(item, QgsProcessingFeedback())
            ]
        )
        assert ids == [1, 2, 3]
    assert worker.deleted() == 0


def errors_file(path: Path, kind: str, wkt: str) -> str:
    """A GeoPackage with a layer of errors that names the first feature of
    `lines`, as the tool writes it, and the source of the layer."""
    name = f'errors_{kind}'
    options = QgsVectorFileWriter.SaveVectorOptions()
    options.driverName = 'GPKG'
    options.layerName = name
    fields = error_fields()
    writer = QgsVectorFileWriter.create(
        str(path),
        fields,
        QgsGeometry.fromWkt(wkt).wkbType(),
        QgsCoordinateReferenceSystem('EPSG:3857'),
        QgsProject.instance().transformContext(),
        options,
    )
    feature = QgsFeature(fields)
    feature.setGeometry(QgsGeometry.fromWkt(wkt))
    feature.setAttribute('rule', 'rule')
    feature.setAttribute('origin_layer', 'lines')
    feature.setAttribute('origin_fid', 1)
    assert writer.addFeatures([feature])
    del writer
    return f'{path}|layername={name}'


def origin_ids(path: str | Path, kind: str) -> list[int]:
    errors = QgsVectorLayer(f'{path}|layername=errors_{kind}', kind, 'ogr')
    return sorted(error['origin_fid'] for error in errors.getFeatures())


def test_writing_the_errors(tmp_path: Path, worker: Worker):
    worker.source = errors_file(
        tmp_path / 'errors.gpkg', 'point', 'MULTIPOINT((0 0))'
    )
    for index in range(ROUNDS):
        path = tmp_path / f'errors{index}.gpkg'
        values = (0, 'rule', 'dangle', 'lines', 7, 0, None, None, None)
        found = Found('point', QgsGeometry.fromWkt('POINT(0 0)'), values)
        worker.after(
            functools.partial(
                write_errors,
                str(path),
                Errors({'point': [found]}, [1], []),
                QgsCoordinateReferenceSystem('EPSG:3857'),
                QgsProcessingContext(),
            )
        )
        assert origin_ids(path, 'point') == [7]
    assert worker.deleted() == 0


def test_adding_the_invalid_features(tmp_path: Path, worker: Worker):
    first = tmp_path / 'errors.gpkg'
    worker.source = errors_file(
        first, 'polygon', 'MULTIPOLYGON(((0 0, 1 0, 0 1, 0 0)))'
    )
    bow = QgsGeometry.fromWkt('POLYGON((0 0, 1 1, 1 0, 0 1, 0 0))')
    invalid = [Invalid('polygons', 4, bow, 'invalid polygon')]
    for index in range(ROUNDS):
        path = shutil.copy(first, tmp_path / f'errors{index}.gpkg')
        worker.after(
            functools.partial(
                append_invalid,
                str(path),
                invalid,
                QgsCoordinateReferenceSystem('EPSG:3857'),
                QgsProcessingContext(),
            )
        )
        assert origin_ids(path, 'polygon') == [1, 4]
    assert worker.deleted() == 0
