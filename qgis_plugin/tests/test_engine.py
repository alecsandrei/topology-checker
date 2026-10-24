"""The engine that runs the checks: the choice of the user, what Automatic
resolves to, a package that is installed while QGIS runs, the log that names
the engine of each run, and a run of the package that is canceled."""

from __future__ import annotations

import collections.abc as c
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
from qgis.PyQt.QtCore import QCoreApplication, QTranslator

from topology_rules.core import binary, engine, folders
from topology_rules.core.checks import Job, run_jobs
from topology_rules.core.engine import Engine, Package
from topology_rules.core.layers import Input
from topology_rules.enums import Setting
from topology_rules.exceptions import ExecutableError, PackageError, RunError
from topology_rules.panel.dock import Dock
from topology_rules.provider import Provider

from .helpers import (
    NEEDS_PACKAGE,
    Ran,
    errors_layer,
    line_layer,
    run_algorithm,
    start_algorithm,
    usable_package,
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


def test_the_choice_is_written_for_the_tasks():
    """A task reads the settings on a thread of its own."""
    engine.choose(Engine.PACKAGE)
    text = Path(QgsSettings().fileName()).read_text(encoding='utf-8')
    assert 'engine=package' in text


def test_choosing_no_engine_lists_the_rules_again(provider: Provider):
    """As after what the engine runs changed."""
    engine.choose(Engine.AUTOMATIC)
    loaded: list[bool] = []
    provider.algorithmsLoaded.connect(lambda: loaded.append(True))
    engine.choose()
    assert loaded == [True]
    assert engine.chosen() is Engine.AUTOMATIC


def test_automatic_is_the_default():
    QgsSettings().remove(Setting.ENGINE.value)
    assert engine.chosen() is Engine.AUTOMATIC


def test_a_choice_that_is_not_known_is_automatic():
    QgsSettings().setValue(Setting.ENGINE.value, 'fastest')
    assert engine.chosen() is Engine.AUTOMATIC


def test_the_executable_is_kept_with_the_package(package: Package):
    engine.choose(Engine.EXECUTABLE)
    assert engine.resolved() is Engine.EXECUTABLE


# Automatic


def test_automatic_is_the_package_where_it_can_be_imported(package: Package):
    engine.choose(Engine.AUTOMATIC)
    assert engine.resolved() is Engine.PACKAGE
    assert engine.package() == package


def test_automatic_is_the_executable_without_the_package(
    monkeypatch: pytest.MonkeyPatch,
):
    without_package(monkeypatch)
    engine.choose(Engine.AUTOMATIC)
    assert engine.package() is None
    assert engine.resolved() is Engine.EXECUTABLE


def test_automatic_is_the_executable_with_a_package_that_is_too_old(
    monkeypatch: pytest.MonkeyPatch,
):
    old = types.ModuleType(engine.MODULE)
    old.__file__ = str(Path('somewhere', engine.MODULE, '__init__.py'))
    monkeypatch.setitem(sys.modules, engine.MODULE, old)
    engine.choose(Engine.AUTOMATIC)
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


def too_old(monkeypatch: pytest.MonkeyPatch) -> None:
    """A package that has none of what the plugin uses."""
    old = types.ModuleType(engine.MODULE)
    old.__file__ = str(Path('somewhere', engine.MODULE, '__init__.py'))
    monkeypatch.setitem(sys.modules, engine.MODULE, old)


def restart_to_load(monkeypatch: pytest.MonkeyPatch, version: str) -> None:
    """The plugin installed the version while QGIS runs an older one."""
    monkeypatch.setattr(folders, 'installed', lambda: version)
    monkeypatch.setattr(folders, 'restart_needed', lambda: True)


def broken(monkeypatch: pytest.MonkeyPatch, folder: Path) -> None:
    """A package whose native part is not there, as a wheel of another
    platform."""
    package = folder / engine.MODULE
    package.mkdir(parents=True)
    (package / '__init__.py').write_text('from . import _native\n')
    for name in list(sys.modules):
        if name.split('.')[0] == engine.MODULE:
            monkeypatch.delitem(sys.modules, name)
    monkeypatch.setattr(sys, 'path', [str(folder), *sys.path])


def brief(error: pytest.ExceptionInfo[PackageError]) -> str:
    return error.value.brief


def test_a_package_that_is_not_there_is_not_installed(
    monkeypatch: pytest.MonkeyPatch,
):
    without_package(monkeypatch)
    with pytest.raises(PackageError) as raised:
        engine.find_package()
    assert brief(raised) == 'not installed'


def test_a_package_that_fails_to_import_does_not_load(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    broken(monkeypatch, tmp_path)
    with pytest.raises(PackageError, match='cannot be imported') as raised:
        engine.find_package()
    assert brief(raised) == 'does not load'


def test_a_package_without_what_the_plugin_uses_is_too_old(
    monkeypatch: pytest.MonkeyPatch,
):
    too_old(monkeypatch)
    with pytest.raises(PackageError) as raised:
        engine.find_package()
    assert brief(raised) == 'too old'


def test_an_old_package_with_a_new_one_installed_needs_a_restart(
    monkeypatch: pytest.MonkeyPatch,
):
    too_old(monkeypatch)
    restart_to_load(monkeypatch, '9.9.9')
    with pytest.raises(
        PackageError, match=r'Restart QGIS to load 9\.9\.9'
    ) as raised:
        engine.find_package()
    assert brief(raised) == 'restart QGIS'


class Marked(QTranslator):
    """Gives each text of the plugin a translation that shows the text."""

    def translate(
        self,
        context: str | None,
        source: str | None,
        disambiguation: str | None = None,
        n: int = -1,
    ) -> str:
        if context == 'TopologyRules':
            return f'<{source}>'
        return source or ''


@pytest.fixture
def marked() -> c.Iterator[None]:
    translator = Marked()
    QCoreApplication.installTranslator(translator)
    yield
    QCoreApplication.removeTranslator(translator)


@pytest.mark.usefixtures('marked')
def test_why_the_package_cannot_be_imported_is_translated(
    monkeypatch: pytest.MonkeyPatch,
):
    without_package(monkeypatch)
    with pytest.raises(PackageError) as raised:
        engine.find_package()
    assert str(raised.value).startswith(
        '<The topology-checker Python package cannot be imported: import of '
    )
    assert str(raised.value).endswith('>')
    assert brief(raised) == '<not installed>'


@pytest.mark.usefixtures('marked')
def test_why_the_package_is_too_old_is_translated(
    monkeypatch: pytest.MonkeyPatch,
):
    too_old(monkeypatch)
    restart_to_load(monkeypatch, '9.9.9')
    with pytest.raises(PackageError) as raised:
        engine.find_package()
    folder = Path('somewhere', engine.MODULE)
    assert str(raised.value) == (
        f'<<The topology-checker Python package at {folder} is too old for '
        'the plugin: it has no Topology.add_layer.> Restart QGIS to load '
        '9.9.9.>'
    )
    assert brief(raised) == '<restart QGIS>'


@pytest.mark.parametrize('needed', [False, True], ids=['skips', 'fails'])
@pytest.mark.parametrize('old', [False, True], ids=['missing', 'too old'])
def test_the_tests_of_the_package_fail_without_it_where_they_need_it(
    monkeypatch: pytest.MonkeyPatch, needed: bool, old: bool
):
    """A run that is to test the package cannot pass by skipping them."""
    if old:
        module = types.ModuleType(engine.MODULE)
        module.__file__ = str(Path('somewhere', engine.MODULE, '__init__.py'))
        monkeypatch.setitem(sys.modules, engine.MODULE, module)
    else:
        without_package(monkeypatch)
    if needed:
        monkeypatch.setenv(NEEDS_PACKAGE, '1')
    else:
        monkeypatch.delenv(NEEDS_PACKAGE, raising=False)
    reason = 'too old' if old else 'cannot be imported'
    with pytest.raises(
        pytest.fail.Exception if needed else pytest.skip.Exception,
        match=reason,
    ):
        usable_package()


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
    engine.choose(Engine.PACKAGE)
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
    engine.choose(Engine.AUTOMATIC)
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
    engine.choose(Engine.PACKAGE)
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
    engine.choose(Engine.PACKAGE)
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
    engine.choose(Engine.PACKAGE)
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
    engine.choose(Engine.PACKAGE)
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
        dock.reload(Engine.EXECUTABLE)
        assert engine.chosen() is Engine.EXECUTABLE
        assert not dock.available
        assert not provider.algorithms()
        dock.reload(Engine.AUTOMATIC)
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
    engine.choose(Engine.PACKAGE)
    lines = line_layer(*segments(20000))
    QgsProject.instance().addMapLayer(lines)
    started = start_algorithm(DANGLES, {'INPUT': lines.id()})
    started.feedback.cancel()
    assert not started.ran().ok
