"""The engine that runs the checks: the choice of the user, what Automatic
resolves to, a package that is installed while QGIS runs, the log that names
the engine of each run, and a run of the package that is canceled."""

from __future__ import annotations

import dataclasses
import importlib.abc
import importlib.machinery
import shutil
import sys
import types
import typing as t
from pathlib import Path

import pytest
from qgis.core import (
    QgsApplication,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
    QgsSettings,
)
from qgis.gui import QgisInterface

from topology_rules.core import binary, engine
from topology_rules.core.checks import Job, run_jobs
from topology_rules.core.engine import Engine, Package
from topology_rules.core.layers import Input
from topology_rules.enums import Setting
from topology_rules.exceptions import ExecutableError, PackageError, RunError
from topology_rules.panel import startup
from topology_rules.panel.dock import Dock
from topology_rules.provider import Provider

from .helpers import (
    Ran,
    choose_engine,
    errors_layer,
    line_layer,
    run_algorithm,
    start_algorithm,
    wait_for,
)

DANGLES = 'line_must_not_have_dangles'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')


def dangles() -> Ran:
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    return run_algorithm(DANGLES, {'INPUT': layer.id()})


def segments(count: int) -> list[str]:
    """Lines that touch no other: two dangles each."""
    return [f'LINESTRING({x} 0, {x + 1} 0)' for x in range(0, 2 * count, 2)]


def without_package(monkeypatch: pytest.MonkeyPatch) -> None:
    """Has the import of the package fail, as where it is not installed."""
    monkeypatch.setitem(sys.modules, engine.MODULE, None)


class Elsewhere(importlib.abc.MetaPathFinder):
    """Finds the package on the path, but not in the folder that it was
    installed in for the tests: as before it was installed there."""

    def __init__(self, folder: Path):
        self.folder = folder

    def find_spec(
        self,
        fullname: str,
        path: t.Sequence[str] | None = None,
        target: types.ModuleType | None = None,
    ) -> importlib.machinery.ModuleSpec | None:
        if fullname != engine.MODULE:
            return None
        found = importlib.machinery.PathFinder.find_spec(
            fullname, [p for p in sys.path if Path(p) != self.folder]
        )
        if found is None:
            raise ModuleNotFoundError(
                f'No module named {fullname!r}', name=fullname
            )
        return found


def hide_package(monkeypatch: pytest.MonkeyPatch, package: Package) -> None:
    """Takes the package away for the test: it is not imported, and it is
    not where it was."""
    monkeypatch.setattr(
        sys, 'meta_path', [Elsewhere(package.location.parent), *sys.meta_path]
    )
    for name in list(sys.modules):
        if name.split('.')[0] == engine.MODULE:
            monkeypatch.delitem(sys.modules, name)


def package_module(violation: type) -> types.ModuleType:
    """A package with all that the plugin uses, and the class of the errors
    that is given."""

    class Topology:
        def add_layer(self) -> None: ...
        def add_rule(self) -> None: ...
        def iter_validate(self) -> None: ...

    module = types.ModuleType(engine.MODULE)
    module.__file__ = str(Path('somewhere', engine.MODULE, '__init__.py'))
    module.__dict__.update(
        Topology=Topology,
        Violation=violation,
        Rule=object(),
        GeometryKind=object(),
        merge_lines=object(),
        explode_lines=object(),
    )
    return module


def install(package: Package, folder: Path) -> None:
    """Puts a copy of the package and of its metadata in the folder, as an
    installer that unpacks a wheel does."""
    for found in package.location.parent.iterdir():
        if found.name == engine.MODULE or (
            found.name.startswith(f'{engine.MODULE}-')
            and found.name.endswith('.dist-info')
        ):
            shutil.copytree(found, folder / found.name)


# The choice


@pytest.mark.parametrize('chosen', list(Engine))
def test_the_choice_is_kept(chosen: Engine):
    engine.choose(chosen)
    assert engine.chosen() is chosen
    assert QgsSettings().value(Setting.ENGINE.value) == chosen.value


def test_automatic_is_the_default():
    QgsSettings().remove(Setting.ENGINE.value)
    assert engine.chosen() is Engine.AUTOMATIC


def test_a_choice_that_is_not_known_is_automatic():
    QgsSettings().setValue(Setting.ENGINE.value, 'fastest')
    assert engine.chosen() is Engine.AUTOMATIC


def test_the_executable_is_kept_with_the_package(package: Package):
    choose_engine(Engine.EXECUTABLE)
    assert engine.resolved() is Engine.EXECUTABLE


# Automatic


def test_automatic_is_the_package_where_it_can_be_imported(package: Package):
    choose_engine(Engine.AUTOMATIC)
    assert engine.resolved() is Engine.PACKAGE
    assert engine.package() == package


def test_automatic_is_the_executable_without_the_package(
    monkeypatch: pytest.MonkeyPatch,
):
    without_package(monkeypatch)
    choose_engine(Engine.AUTOMATIC)
    assert engine.package() is None
    assert engine.resolved() is Engine.EXECUTABLE


def test_automatic_is_the_executable_with_a_package_that_is_too_old(
    monkeypatch: pytest.MonkeyPatch,
):
    old = types.ModuleType(engine.MODULE)
    old.__file__ = str(Path('somewhere', engine.MODULE, '__init__.py'))
    monkeypatch.setitem(sys.modules, engine.MODULE, old)
    choose_engine(Engine.AUTOMATIC)
    assert engine.resolved() is Engine.EXECUTABLE
    with pytest.raises(
        PackageError, match=r'too old .* no Topology\.add_layer'
    ):
        engine.find_package()


def test_a_package_that_gives_no_wkb_of_the_errors_is_too_old(
    monkeypatch: pytest.MonkeyPatch,
):
    """The package before 0.3 gives the errors as shapely geometries only."""

    @dataclasses.dataclass
    class Violation:
        geometry: object

    monkeypatch.setitem(sys.modules, engine.MODULE, package_module(Violation))
    with pytest.raises(PackageError, match=r'too old .* no Violation\.wkb'):
        engine.find_package()


@pytest.mark.parametrize('field', [True, False], ids=['field', 'property'])
def test_the_wkb_of_the_errors_is_found(
    monkeypatch: pytest.MonkeyPatch, field: bool
):
    """A field without a default is not an attribute of its class."""
    if field:

        @dataclasses.dataclass
        class Violation:
            wkb: bytes

    else:

        class Violation:
            @property
            def wkb(self) -> bytes:
                return b''

    monkeypatch.setitem(sys.modules, engine.MODULE, package_module(Violation))
    found = engine.find_package()
    assert found.location == Path('somewhere', engine.MODULE)


def test_the_package_chosen_without_it_says_why(
    monkeypatch: pytest.MonkeyPatch,
):
    without_package(monkeypatch)
    choose_engine(Engine.PACKAGE)
    assert engine.resolved() is Engine.PACKAGE
    with pytest.raises(PackageError, match='cannot be imported'):
        engine.tool()


def test_a_run_of_the_package_chosen_without_it_fails(
    executable: str, provider: Provider, monkeypatch: pytest.MonkeyPatch
):
    """The executable does not stand in for it. Choosing the package lists
    its rules instead, so the setting changes here as another window of the
    same profile would change it."""
    without_package(monkeypatch)
    settings = QgsSettings()
    settings.setValue(Setting.ENGINE.value, Engine.PACKAGE.value)
    settings.sync()
    ran = dangles()
    assert not ran.ok
    assert 'Engine: executable' not in ran.log


def test_a_package_installed_while_qgis_runs_is_used(
    executable: str,
    provider: Provider,
    package: Package,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
):
    """The folder that an installer fills is on the path before the package
    is in it, and the package is used once it is, without a restart."""
    hide_package(monkeypatch, package)
    folder = tmp_path / 'dependencies'
    monkeypatch.setattr(sys, 'path', [str(folder), *sys.path])
    choose_engine(Engine.AUTOMATIC)
    assert engine.package() is None
    before = dangles()
    assert before.ok, before.log
    assert before.log.startswith(f'Engine: executable {executable}')

    folder.mkdir()
    install(package, folder)
    assert engine.package() == Package(package.version, folder / engine.MODULE)
    after = dangles()
    assert after.ok, after.log
    assert after.log.startswith(
        f'Engine: Python package topology-checker {package.version} '
        f'({folder / engine.MODULE})'
    )
    assert after.results['COUNT'] == before.results['COUNT'] == 2


# The log


def test_the_log_names_the_executable_and_the_command(
    executable: str, provider: Provider
):
    wait_for(lambda: bool(binary.version(executable)))
    ran = dangles()
    assert ran.ok, ran.log
    lines = ran.log.splitlines()
    assert lines[0] == (
        f'Engine: executable {executable} {binary.version(executable)}'
    )
    assert lines[1].startswith(f'{executable} --json validate '), ran.log


def test_the_log_names_the_package(
    executable: str, provider: Provider, package: Package
):
    choose_engine(Engine.PACKAGE)
    ran = dangles()
    assert ran.ok, ran.log
    assert ran.log.splitlines()[0] == (
        f'Engine: Python package topology-checker {package.version} '
        f'({package.location})'
    )
    assert executable not in ran.log


def test_no_executable_is_looked_for_with_the_package(
    executable: str,
    provider: Provider,
    package: Package,
    monkeypatch: pytest.MonkeyPatch,
):
    def unused() -> Path:
        raise ExecutableError('The executable was looked for')

    monkeypatch.setattr(binary, 'find_executable', unused)
    choose_engine(Engine.PACKAGE)
    ran = dangles()
    assert ran.ok, ran.log
    assert ran.results['COUNT'] == 2


@pytest.mark.parametrize(
    ('name', 'output', 'count'),
    [(DANGLES, 'ERRORS_POINT', 2), ('merge_lines', 'OUTPUT', 1)],
)
def test_the_package_runs_without_shapely_and_pyproj(
    provider: Provider,
    package: Package,
    monkeypatch: pytest.MonkeyPatch,
    name: str,
    output: str,
    count: int,
):
    """QGIS may have neither, and the plugin gives the package WKB."""
    for module in list(sys.modules):
        if module.split('.')[0] in {'shapely', 'pyproj'}:
            monkeypatch.delitem(sys.modules, module)
    monkeypatch.setitem(sys.modules, 'shapely', None)
    monkeypatch.setitem(sys.modules, 'pyproj', None)
    choose_engine(Engine.PACKAGE)
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    ran = run_algorithm(name, {'INPUT': layer.id(), output: 'memory:'})
    assert ran.ok, ran.log
    assert errors_layer(ran, output).featureCount() == count


# Without an executable


@pytest.fixture
def package_only(
    package: Package, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> Package:
    """The package, and no executable to be found."""
    monkeypatch.setenv('PATH', str(tmp_path / 'nothing'))
    choose_engine(Engine.PACKAGE)
    return package


def test_the_dock_lists_the_rules_of_the_package(
    qgis_iface: QgisInterface, package_only: Package
):
    dock = Dock(qgis_iface)
    try:
        dock.refresh()
        assert dock.available
        assert dock.setup_page.isHidden()
        assert 'line/must-not-have-dangles' in {r.name for r in dock.form.rules}
    finally:
        dock.deleteLater()


def test_the_toolbox_lists_the_rules_of_the_package(
    qgis_processing: None, package_only: Package
):
    provider = Provider()
    registry = QgsApplication.processingRegistry()
    registry.addProvider(provider)
    try:
        names = {algorithm.name() for algorithm in provider.algorithms()}
        assert {DANGLES, 'check_rules', 'merge_lines'} <= names
    finally:
        registry.removeProvider(provider)


def test_switching_the_engine_lists_its_rules(
    qgis_iface: QgisInterface,
    qgis_processing: None,
    package_only: Package,
):
    provider = Provider()
    registry = QgsApplication.processingRegistry()
    registry.addProvider(provider)
    dock = Dock(qgis_iface)
    try:
        dock.refresh()
        assert dock.available
        # There is no executable to switch to
        startup.switch(dock, Engine.EXECUTABLE)
        assert engine.chosen() is Engine.EXECUTABLE
        assert not dock.available
        assert not provider.algorithms()
        startup.switch(dock, Engine.AUTOMATIC)
        assert dock.available
        assert provider.algorithms()
    finally:
        dock.deleteLater()
        registry.removeProvider(provider)


# Canceling


def test_a_run_of_the_package_stops_between_two_errors(
    package: Package, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    """The errors come from a stream, which is closed when the run is
    canceled, before all of them are found."""
    from topology_rules.core import package as runner

    lines = line_layer(*segments(1000))
    QgsProject.instance().addMapLayer(lines)
    rules = {rule.name: rule for rule in engine.rules(package)}
    job = Job(
        rules['line/must-not-have-dangles'], Input(lines.name(), lines, lines)
    )
    feedback = QgsProcessingFeedback()
    seen: list[int] = []
    attributes = runner.attributes

    def cancel_at_first(index: int, *rest: t.Any) -> list[object]:
        seen.append(index)
        feedback.cancel()
        return attributes(index, *rest)

    monkeypatch.setattr(runner, 'attributes', cancel_at_first)
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    output = tmp_path / 'errors.gpkg'
    with pytest.raises(RunError, match='Canceled'):
        run_jobs([job], package, None, str(output), tmp_path, context, feedback)
    assert seen == [0]
    assert not output.exists()


def test_a_canceled_run_of_the_package_ends(
    executable: str, provider: Provider, package: Package
):
    choose_engine(Engine.PACKAGE)
    lines = line_layer(*segments(20000))
    QgsProject.instance().addMapLayer(lines)
    started = start_algorithm(DANGLES, {'INPUT': lines.id()})
    started.feedback.cancel()
    assert not started.ran().ok
