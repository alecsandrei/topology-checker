"""QGIS 3.34 on Ubuntu 22.04 runs Python 3.10, which has no tomllib and none of
the syntax and library names that came after it."""

from __future__ import annotations

import ast
import collections.abc as c
import importlib
import sys
from pathlib import Path

import pytest
from qgis.core import QgsApplication
from qgis.gui import QgisInterface

import topology_rules

from .helpers import (
    message_text,
    run_algorithm,
    saved_layer,
    wait_for,
    write_toml,
)

ROOT = Path(topology_rules.__file__).parent
MODULES = sorted(ROOT.rglob('*.py'))
PACKAGE = 'topology_rules'
DANGLES = 'line/must-not-have-dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
# Names that Python 3.11 and 3.12 added to the standard library
LATE = {
    'typing': {
        'Self',
        'Never',
        'LiteralString',
        'Required',
        'NotRequired',
        'TypeVarTuple',
        'Unpack',
        'assert_never',
        'assert_type',
        'reveal_type',
        'override',
        'dataclass_transform',
    },
    'enum': {'StrEnum', 'ReprEnum', 'verify'},
    'datetime': {'UTC'},
    'itertools': {'batched'},
    'asyncio': {'TaskGroup', 'timeout', 'Runner'},
}


def name_of(path: Path) -> str:
    return str(path.relative_to(ROOT))


@pytest.mark.parametrize('path', MODULES, ids=name_of)
def test_every_module_has_the_syntax_of_python_3_10(path: Path):
    ast.parse(
        path.read_text(encoding='utf-8'),
        filename=str(path),
        feature_version=(3, 10),
    )


@pytest.mark.parametrize('path', MODULES, ids=name_of)
def test_no_module_imports_a_name_that_python_3_10_lacks(path: Path):
    tree = ast.parse(path.read_text(encoding='utf-8'))
    late: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom) and node.module in LATE:
            late += [
                f'{node.module}.{alias.name}'
                for alias in node.names
                if alias.name in LATE[node.module]
            ]
    assert late == []


# Without tomllib, as on Python 3.10


@pytest.fixture
def without_tomllib(monkeypatch: pytest.MonkeyPatch) -> None:
    """Python imports the plugin again, on an interpreter that has no
    tomllib."""
    for name in [
        name
        for name in sys.modules
        if name == PACKAGE or name.startswith(f'{PACKAGE}.')
    ]:
        monkeypatch.delitem(sys.modules, name)
    monkeypatch.setitem(sys.modules, 'tomllib', None)
    # The plugin may use the backport where it is installed
    monkeypatch.setitem(sys.modules, 'tomli', None)


@pytest.mark.parametrize(
    'module',
    [
        PACKAGE,
        f'{PACKAGE}.plugin',
        f'{PACKAGE}.provider',
        f'{PACKAGE}.algorithms',
        f'{PACKAGE}.panel.dock',
    ],
)
def test_the_plugin_imports_without_tomllib(without_tomllib: None, module: str):
    importlib.import_module(module)


def test_the_dock_opens_without_tomllib(
    without_tomllib: None,
    qgis_iface: QgisInterface,
    executable: str,
    qgis_processing: None,
):
    dock = importlib.import_module(f'{PACKAGE}.panel.dock').Dock(qgis_iface)
    dock.refresh()
    wait_for(lambda: dock.probing is None)
    assert dock.available
    dock.deleteLater()


def test_a_definition_cannot_be_read_without_tomllib_and_says_so(
    without_tomllib: None,
    qgis_iface: QgisInterface,
    executable: str,
    qgis_processing: None,
    tmp_path: Path,
    bar: object,
):
    dock = importlib.import_module(f'{PACKAGE}.panel.dock').Dock(qgis_iface)
    dock.refresh()
    wait_for(lambda: dock.probing is None)
    file = write_toml(
        tmp_path / 'topology.toml',
        {'roads': ('roads.gpkg', 'roads')},
        [(DANGLES, 'roads', None)],
    )
    dock.import_rules(file)
    assert dock.entries == []
    assert 'toml' in message_text(bar).lower()
    dock.deleteLater()


def test_the_provider_lists_its_algorithms_without_tomllib(
    without_tomllib: None, executable: str, qgis_processing: None
):
    provider = importlib.import_module(f'{PACKAGE}.provider').Provider()
    registry = QgsApplication.processingRegistry()
    registry.addProvider(provider)
    try:

        def names() -> c.Iterable[str]:
            return {algorithm.name() for algorithm in provider.algorithms()}

        wait_for(lambda: 'check_definition' in names(), 30)
    finally:
        wait_for(lambda: not QgsApplication.taskManager().activeTasks(), 30)
        registry.removeProvider(provider)


def test_a_definition_is_run_without_tomllib(
    without_tomllib: None,
    executable: str,
    qgis_processing: None,
    tmp_path: Path,
):
    # The algorithm cannot read the file to see which files it names, so it
    # has no check of the outputs, and the tool reads the file
    provider = importlib.import_module(f'{PACKAGE}.provider').Provider()
    registry = QgsApplication.processingRegistry()
    registry.addProvider(provider)
    try:
        wait_for(lambda: provider.algorithm('check_definition') is not None, 30)
        saved_layer(tmp_path, CHAIN, 'roads')
        definition = write_toml(
            tmp_path / 'topology.toml',
            {'roads': ('roads.gpkg', 'roads')},
            [(DANGLES, 'roads', None)],
        )
        ran = run_algorithm('check_definition', {'DEFINITION': str(definition)})
        assert ran.ok, ran.log
        assert ran.results['COUNT'] == len(CHAIN)
    finally:
        wait_for(lambda: not QgsApplication.taskManager().activeTasks(), 30)
        registry.removeProvider(provider)
