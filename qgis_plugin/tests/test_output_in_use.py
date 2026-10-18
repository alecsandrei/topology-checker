"""The file that the errors are written to may be open: Windows does not let a
program replace a file that another has open, such as the layers of the errors
of the run before. The run frees the file when it can, and else writes beside
it. A file that cannot be replaced is made by making `Path.unlink` refuse, so
that this is tested everywhere."""

from __future__ import annotations

import collections.abc as c
import json
import pathlib
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsProject, QgsVectorLayer

from topology_rules.algorithms.check import COUNT, OUTPUT
from topology_rules.provider import Provider

from .helpers import Ran, line_layer, run_algorithm

DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


@pytest.fixture(autouse=True)
def ready(executable: str, provider: Provider) -> None:
    """The real executable and the provider, as in a QGIS session."""


@pytest.fixture
def in_use(monkeypatch: pytest.MonkeyPatch) -> c.Callable[..., None]:
    """Makes the files with the given names refuse to be deleted."""
    original = pathlib.Path.unlink

    def hold(*names: str) -> None:
        def unlink(self: Path, *args: t.Any, **kwargs: t.Any) -> None:
            if any(self.name.startswith(name) for name in names):
                raise PermissionError(13, 'The file is being used', str(self))
            original(self, *args, **kwargs)

        monkeypatch.setattr(pathlib.Path, 'unlink', unlink)

    return hold


def run(layer: QgsVectorLayer, output: Path) -> Ran:
    return run_algorithm(
        'check_rules',
        {
            'RULES': json.dumps(
                [{'rule': DANGLES, 'layer': layer.id(), 'other': None}]
            ),
            'OUTPUT': str(output),
        },
    )


def lines() -> QgsVectorLayer:
    layer = line_layer(*CHAIN, name='lines')
    QgsProject.instance().addMapLayer(layer)
    return layer


def errors_in(path: str) -> int:
    layer = QgsVectorLayer(f'{path}|layername=errors_point', 'e', 'ogr')
    return layer.featureCount() if layer.isValid() else 0


def test_a_file_that_is_free_is_written_where_it_was_asked(tmp_path: Path):
    output = tmp_path / 'errors.gpkg'
    output.write_bytes(b'old')
    ran = run(lines(), output)
    assert ran.ok, ran.log
    assert ran.results[OUTPUT] == str(output)


def test_a_file_that_is_in_use_is_not_replaced(
    tmp_path: Path, in_use: c.Callable[..., None]
):
    output = tmp_path / 'errors.gpkg'
    output.write_bytes(b'old')
    in_use('errors.gpkg')
    ran = run(lines(), output)
    assert ran.ok, ran.log
    assert output.read_bytes() == b'old'


def test_the_errors_are_written_beside_a_file_that_is_in_use(
    tmp_path: Path, in_use: c.Callable[..., None]
):
    output = tmp_path / 'errors.gpkg'
    output.write_bytes(b'old')
    in_use('errors.gpkg')
    ran = run(lines(), output)
    assert ran.ok, ran.log
    assert ran.results[OUTPUT] == str(tmp_path / 'errors_2.gpkg')
    assert errors_in(str(ran.results[OUTPUT])) == 2
    assert ran.results[COUNT] == 2


def test_the_log_says_that_the_file_was_in_use_and_where_the_errors_are(
    tmp_path: Path, in_use: c.Callable[..., None]
):
    output = tmp_path / 'errors.gpkg'
    output.write_bytes(b'old')
    in_use('errors.gpkg')
    ran = run(lines(), output)
    assert 'in use' in ran.log
    assert 'errors_2.gpkg' in ran.log


def test_the_next_name_is_tried_when_that_one_is_in_use_too(
    tmp_path: Path, in_use: c.Callable[..., None]
):
    output = tmp_path / 'errors.gpkg'
    output.write_bytes(b'old')
    in_use('errors.gpkg', 'errors_2.gpkg')
    ran = run(lines(), output)
    assert ran.ok, ran.log
    assert ran.results[OUTPUT] == str(tmp_path / 'errors_3.gpkg')


def test_the_files_of_the_journal_count_as_in_use_too(
    tmp_path: Path, in_use: c.Callable[..., None]
):
    output = tmp_path / 'errors.gpkg'
    output.write_bytes(b'old')
    in_use('errors.gpkg-wal')
    ran = run(lines(), output)
    assert ran.ok, ran.log
    assert ran.results[OUTPUT] == str(tmp_path / 'errors_2.gpkg')


def test_a_run_gives_up_when_no_name_is_free_and_names_the_file(
    tmp_path: Path, in_use: c.Callable[..., None]
):
    output = tmp_path / 'errors.gpkg'
    output.write_bytes(b'old')
    in_use('errors')
    ran = run(lines(), output)
    assert not ran.ok
    assert 'errors.gpkg' in ran.log


# The layers of the project that read the file


def test_a_layer_of_the_project_that_reads_the_output_is_let_go_of(
    tmp_path: Path,
):
    layer = lines()
    output = tmp_path / 'errors.gpkg'
    first = run(layer, output)
    assert first.ok, first.log
    reading = QgsVectorLayer(
        f'{output}|layername=errors_point', 'errors', 'ogr'
    )
    assert reading.isValid()
    QgsProject.instance().addMapLayer(reading)
    identifier = reading.id()
    second = run(layer, output)
    assert second.ok, second.log
    assert QgsProject.instance().mapLayer(identifier) is None


def test_a_layer_of_the_project_that_reads_another_file_is_kept(
    tmp_path: Path,
):
    layer = lines()
    other = tmp_path / 'other.gpkg'
    assert run(layer, other).ok
    reading = QgsVectorLayer(f'{other}|layername=errors_point', 'o', 'ogr')
    QgsProject.instance().addMapLayer(reading)
    identifier = reading.id()
    assert run(layer, tmp_path / 'errors.gpkg').ok
    assert QgsProject.instance().mapLayer(identifier) is not None
