"""What runs the checks: the topology-checker Python package inside QGIS, or
the executable in a process of its own.

The package is imported only when it is asked for, never as the plugin
loads, so that a package installed while QGIS runs is found then.
"""

from __future__ import annotations

import dataclasses
import functools
import importlib
import importlib.metadata
import typing as t
from enum import Enum
from pathlib import Path

from qgis.core import QgsApplication, QgsSettings

from ..enums import PROVIDER_ID, Setting
from ..exceptions import PackageError
from . import binary, folders

if t.TYPE_CHECKING:
    from .binary import Rule

# The distribution of the package, and the module that it installs
DISTRIBUTION = 'topology-checker'
MODULE = 'topology_checker'
# What the plugin uses of the package, which an older one lacks: the plugin
# gives it WKB and reads the WKB of the errors, without shapely or pyproj
NEEDED = (
    'Topology.add_layer',
    'Topology.add_rule',
    'Topology.iter_validate',
    'Violation.wkb',
    'Rule',
    'GeometryKind',
    'merge_lines',
    'explode_lines',
)


class Engine(str, Enum):
    """The engine that the user chose. Automatic is the package where QGIS
    can use it, else the executable."""

    AUTOMATIC = 'automatic'
    PACKAGE = 'package'
    EXECUTABLE = 'executable'


@dataclasses.dataclass(frozen=True)
class Package:
    """The topology-checker Python package that QGIS imports."""

    version: str
    location: Path


# What a run uses: the package, or the path of the executable
Tool = Package | str


def chosen() -> Engine:
    """The engine that the user chose, Automatic by default."""
    value = QgsSettings().value(Setting.ENGINE.value, Engine.AUTOMATIC.value)
    try:
        return Engine(str(value))
    except ValueError:
        return Engine.AUTOMATIC


def choose(engine: Engine) -> None:
    """Has the engine run the checks from now on, and the toolbox list its
    rules."""
    QgsSettings().setValue(Setting.ENGINE.value, engine.value)
    registry = QgsApplication.processingRegistry()
    provider = None if registry is None else registry.providerById(PROVIDER_ID)
    if provider is not None:
        provider.refreshAlgorithms()


def missing(module: object) -> str | None:
    """The first thing the plugin needs that the module does not have. A
    field of a dataclass counts, which its class has only with a default."""
    for name in NEEDED:
        found = module
        for part in name.split('.'):
            fields = getattr(found, '__dataclass_fields__', {})
            found = getattr(found, part, fields.get(part))
        if found is None:
            return name
    return None


@functools.cache
def version_at(location: Path) -> str:
    """The version of the package in the folder, from the metadata that is
    installed beside it, or empty where there is none. It is the version of
    the package that was imported from there, which stays until QGIS
    restarts, so it is read once."""
    for found in importlib.metadata.distributions(
        name=DISTRIBUTION, path=[str(location.parent)]
    ):
        return str(found.version)
    return ''


def find_package() -> Package:
    """The package, or a PackageError that says why QGIS cannot use it."""
    # A package installed since the last import is found
    importlib.invalidate_caches()
    try:
        module = importlib.import_module(MODULE)
    except ImportError as error:
        raise PackageError(
            f'The topology-checker Python package cannot be imported: {error}'
        ) from error
    location = Path(str(module.__file__)).parent
    lacking = missing(module)
    if lacking is not None:
        message = (
            f'The topology-checker Python package at {location} is too old '
            f'for the plugin: it has no {lacking}.'
        )
        # The one that was installed while QGIS runs loads at the next start
        installed = folders.installed()
        if installed is not None and folders.restart_needed():
            message += f' Restart QGIS to load {installed}.'
        raise PackageError(message)
    return Package(version_at(location), location)


def package() -> Package | None:
    """The package where QGIS can import and use it, else none."""
    try:
        return find_package()
    except PackageError:
        return None


def resolved() -> Engine:
    """The engine that runs the checks now: the package or the executable."""
    engine = chosen()
    if engine is Engine.AUTOMATIC:
        return Engine.EXECUTABLE if package() is None else Engine.PACKAGE
    return engine


def tool() -> Tool:
    """What a run uses, which reads the settings and so belongs to the main
    thread. Raises an ExecutableError, or a PackageError, when the engine
    cannot be used."""
    if resolved() is Engine.PACKAGE:
        return find_package()
    return str(binary.find_executable())


def package_rules() -> tuple[Rule, ...]:
    """The rules of the package, which raises a PackageError when it cannot
    be used."""
    return rules(find_package())


def rules(used: Tool) -> tuple[Rule, ...]:
    """The rules that the package or the executable knows."""
    if isinstance(used, Package):
        from . import package as runner

        return runner.rules()
    return binary.rules()


def describe(used: Tool) -> str:
    """The engine of a run, as the log names it."""
    if isinstance(used, Package):
        version = f' {used.version}' if used.version else ''
        return (
            f'Engine: Python package {DISTRIBUTION}{version} ({used.location})'
        )
    version = binary.version(used)
    return f'Engine: executable {used}{f" {version}" if version else ""}'
