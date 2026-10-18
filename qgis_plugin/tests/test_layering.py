"""How the plugin is layered: the core knows nothing of the panel or of
widgets, so that it can be used and tested without them."""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

import topology_rules

PLUGIN = Path(topology_rules.__file__).parent
CORE = PLUGIN / 'core'
# What the core must not use: the panel, and anything that draws
FORBIDDEN = (
    'topology_rules.panel',
    'qgis.gui',
    'qgis.PyQt.QtWidgets',
    'PyQt5.QtWidgets',
    'PyQt6.QtWidgets',
)


def imported(source: str, package: str) -> set[str]:
    """The modules that the source imports, with the relative ones made
    absolute from the package that holds it, and the names of a
    `from x import y` as `x.y` as well, since y may be a module."""
    names: set[str] = set()
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Import):
            names.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            base = node.module or ''
            if node.level:
                parts = package.split('.')[
                    : len(package.split('.')) - node.level + 1
                ]
                base = '.'.join(
                    [*parts, *([node.module] if node.module else [])]
                )
            names.add(base)
            names.update(f'{base}.{alias.name}' for alias in node.names)
    return names


def forbidden(names: set[str]) -> list[str]:
    return sorted(
        name
        for name in names
        if any(name == bad or name.startswith(f'{bad}.') for bad in FORBIDDEN)
    )


def package_of(path: Path) -> str:
    relative = path.relative_to(PLUGIN.parent).parent
    return '.'.join(relative.parts)


def core_modules() -> list[Path]:
    return sorted(CORE.rglob('*.py'))


# The checker itself


@pytest.mark.parametrize(
    ('source', 'package', 'found'),
    [
        ('import qgis.gui', 'topology_rules.core', ['qgis.gui']),
        (
            'from qgis.gui import QgisInterface',
            'topology_rules.core',
            ['qgis.gui', 'qgis.gui.QgisInterface'],
        ),
        (
            'from qgis.PyQt.QtWidgets import QLabel',
            'topology_rules.core',
            ['qgis.PyQt.QtWidgets', 'qgis.PyQt.QtWidgets.QLabel'],
        ),
        (
            'from ..panel import dock',
            'topology_rules.core',
            ['topology_rules.panel', 'topology_rules.panel.dock'],
        ),
        (
            'from .. import panel',
            'topology_rules.core',
            ['topology_rules.panel'],
        ),
        (
            'from ...panel.dock import Dock',
            'topology_rules.core.sub',
            ['topology_rules.panel.dock', 'topology_rules.panel.dock.Dock'],
        ),
        (
            'from topology_rules.panel import dock',
            'x',
            ['topology_rules.panel', 'topology_rules.panel.dock'],
        ),
        (
            'if False:\n    import qgis.gui',
            'topology_rules.core',
            ['qgis.gui'],
        ),
    ],
)
def test_the_checker_finds_what_the_core_must_not_use(
    source: str, package: str, found: list[str]
):
    assert forbidden(imported(source, package)) == found


@pytest.mark.parametrize(
    ('source', 'package'),
    [
        ('import qgis.core', 'topology_rules.core'),
        ('from qgis.core import QgsProject', 'topology_rules.core'),
        ('from qgis.PyQt.QtCore import QObject', 'topology_rules.core'),
        ('from .binary import Rule', 'topology_rules.core'),
        ('from ..algorithms import base', 'topology_rules.core'),
        ('import qgis.guix', 'topology_rules.core'),
    ],
)
def test_the_checker_lets_the_rest_through(source: str, package: str):
    assert forbidden(imported(source, package)) == []


# The plugin


def test_the_plugin_has_a_core():
    assert CORE.is_dir(), 'topology_rules/core is the part without widgets'
    assert (CORE / '__init__.py').is_file()
    assert len(core_modules()) > 1


def test_the_plugin_has_a_panel():
    assert (PLUGIN / 'panel' / '__init__.py').is_file()


def test_the_core_does_not_use_the_panel_nor_widgets():
    used = {
        str(path.relative_to(PLUGIN)): forbidden(
            imported(path.read_text(encoding='utf-8'), package_of(path))
        )
        for path in core_modules()
    }
    assert {name: found for name, found in used.items() if found} == {}


@pytest.mark.parametrize('path', core_modules(), ids=lambda p: p.name)
def test_a_module_of_the_core_imports_on_its_own(path: Path):
    """Importing it does not load the panel."""
    import importlib
    import sys

    module = '.'.join(path.relative_to(PLUGIN.parent).with_suffix('').parts)
    module = module.removesuffix('.__init__')
    before = {
        name for name in sys.modules if name.startswith('topology_rules.panel')
    }
    importlib.import_module(module)
    after = {
        name for name in sys.modules if name.startswith('topology_rules.panel')
    }
    assert after <= before
