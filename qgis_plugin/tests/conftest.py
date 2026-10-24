"""Fixtures of the tests: the executable, the Python package, the provider,
the dock, the message bar, and a clean project and settings for each test."""

from __future__ import annotations

import atexit
import collections.abc as c
import contextlib
import os
import shutil
import sys
import tempfile
import threading
import traceback
import types
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsApplication, QgsProject, QgsSettings
from qgis.gui import QgisInterface
from qgis.PyQt.QtCore import QSettings
from qgis.PyQt.QtGui import QFontDatabase, QFontInfo
from qgis.PyQt.QtWidgets import QApplication, QMainWindow

from topology_rules.core import binary, engine, install
from topology_rules.core.engine import Engine, Package
from topology_rules.enums import Setting
from topology_rules.exceptions import ExecutableError, PackageError
from topology_rules.panel.dock import Dock
from topology_rules.panel.rules_page import COLLAPSED
from topology_rules.provider import Provider
from topology_rules.utils import not_none

from .helpers import choose_engine, real_executable, wait_for


@pytest.hookimpl(trylast=True)
def pytest_configure(config: pytest.Config) -> None:
    """Gives the settings of QGIS a folder for this run alone.

    pytest-qgis keeps them in `.qgis-settings` of the project, which every
    run from the project shares, and so shares the path of the executable
    that a test sets. This runs after pytest-qgis has chosen its folder.
    """
    folder = tempfile.mkdtemp(prefix='topology-rules-settings-')
    atexit.register(shutil.rmtree, folder, ignore_errors=True)
    QSettings.setPath(
        QSettings.Format.IniFormat, QSettings.Scope.UserScope, folder
    )


# The size of the text of a user interface, in pixels: 9 points at 96 dpi,
# which is what QGIS has on Windows, Linux and macOS
UI_TEXT_PIXELS = 12


# The family of the text of QGIS on Windows, and the file that it is in
WINDOWS_FONT = 'Segoe UI'
WINDOWS_FONT_FILE = 'segoeui.ttf'


@pytest.fixture(autouse=True, scope='session')
def ui_font(qgis_app: QgsApplication) -> str | None:
    """Gives the widgets the text that a user has. Without a window system a
    platform picks a font of its own: Windows without a screen has Helvetica,
    which it does not have, at 12 points and 100 dpi, and Qt then draws with a
    much wider font, which makes a panel wider than it is for a person. So the
    family is the one of QGIS on Windows, and elsewhere that of the system, and
    the size is in pixels, which do not depend on the dpi: 12 of them are 9
    points at 96 dpi. What the layout tests measure is then much the same on
    Linux, macOS and Windows. The tests of a large font set their own size.

    A platform without a screen does not load the fonts of Windows, so the file
    of the font is added to Qt by hand, which works on every platform.

    Returns why the font is not the one that was asked for, or none. The
    tests of the layout in pixels are skipped for a font that is not there."""
    font = QApplication.font()
    problem = ''
    if sys.platform == 'win32':
        folder = Path(os.environ.get('WINDIR', r'C:\Windows')) / 'Fonts'
        file = folder / WINDOWS_FONT_FILE
        if QFontDatabase.addApplicationFont(str(file)) == -1:
            problem = f' ({file} could not be loaded)'
        font.setFamily(WINDOWS_FONT)
    font.setPixelSize(UI_TEXT_PIXELS)
    QApplication.setFont(font)
    if sys.platform != 'win32':
        return None
    drawn = QFontInfo(QApplication.font())
    if drawn.family() == WINDOWS_FONT and drawn.exactMatch():
        return None
    return (
        f'{WINDOWS_FONT} is not here, and {drawn.family()!r} is drawn '
        f'instead{problem}'
    )


@pytest.fixture
def real_font(ui_font: str | None) -> None:
    """Skips a test of the layout in pixels when the font is not the one of
    the user, as the measures of another font say nothing about the panel."""
    if ui_font is not None:
        pytest.skip(ui_font)


def forget_options_box(settings: QgsSettings) -> None:
    """The box of options is closed in a new profile: the plugin keeps its
    state, and QGIS keeps that of a box with a name."""
    settings.remove(COLLAPSED)
    for key in settings.allKeys():
        if 'TopologyRulesOptions' in key:
            settings.remove(key)
    settings.sync()


def forget_tools() -> None:
    """Forgets the rules of the tools that were asked. The real tool is the
    same file in every test, so what it said is kept, and each test does not
    start it again: that costs a process, which on Windows is slow. The fake
    tools are files of a test, and are forgotten."""
    real = real_executable()
    probe = binary._rules
    with probe.lock:
        rules = {k: v for k, v in probe.done.items() if k[0] == real}
        formats = {k: v for k, v in probe.readable.items() if k[0] == real}
        versions = {k: v for k, v in probe.versions.items() if k[0] == real}
    probe.cache_clear()
    with probe.lock:
        probe.done.update(rules)
        probe.readable.update(formats)
        probe.versions.update(versions)


@pytest.fixture(autouse=True)
def no_executable(monkeypatch: pytest.MonkeyPatch) -> c.Iterator[None]:
    """Starts without a configured executable and without cached rules."""
    # The fake executables are scripts for the system Python, which a
    # bundled QGIS Python home would break
    monkeypatch.delenv('PYTHONHOME', raising=False)
    monkeypatch.delenv('PYTHONPATH', raising=False)
    settings = QgsSettings()
    settings.remove(Setting.EXECUTABLE.value)
    forget_options_box(settings)
    forget_tools()
    yield
    settings.remove(Setting.EXECUTABLE.value)
    forget_options_box(settings)
    forget_tools()


@pytest.fixture(autouse=True)
def executable_engine() -> c.Iterator[None]:
    """Runs the checks with the executable, which most tests are about. The
    tests of the package, and those of both engines, choose theirs."""
    choose_engine(Engine.EXECUTABLE)
    yield
    QgsSettings().remove(Setting.ENGINE.value)


@pytest.fixture
def package() -> Package:
    """The Python package, skipping, with the reason, where QGIS cannot use
    it."""
    try:
        return engine.find_package()
    except PackageError as error:
        pytest.skip(str(error))


def executable_unused() -> Path:
    raise ExecutableError('The package runs the checks, not the executable')


@pytest.fixture(
    params=[Engine.EXECUTABLE, Engine.PACKAGE],
    ids=[Engine.EXECUTABLE.value, Engine.PACKAGE.value],
)
def run_engine(
    request: pytest.FixtureRequest, monkeypatch: pytest.MonkeyPatch
) -> Engine:
    """Runs the checks of a test once with the executable and once with the
    package. With the package no executable is found, as for a user who has
    only the package."""
    chosen = t.cast(Engine, request.param)
    if chosen is Engine.PACKAGE:
        request.getfixturevalue('package')
        monkeypatch.setattr(binary, 'find_executable', executable_unused)
    choose_engine(chosen)
    return chosen


@pytest.fixture(autouse=True)
def unhandled(monkeypatch: pytest.MonkeyPatch) -> c.Iterator[list[str]]:
    """Fails a test when a slot or task callback raised.

    PyQt turns such an exception into an abort of the whole process, or
    prints it to the QGIS Python error panel, where a user sees it.
    """
    errors: list[str] = []

    def hook(
        kind: type[BaseException],
        value: BaseException,
        trace: types.TracebackType | None,
    ) -> None:
        errors.append(''.join(traceback.format_exception(kind, value, trace)))

    monkeypatch.setattr(sys, 'excepthook', hook)
    yield errors
    assert not errors, '\n'.join(errors)


def installing() -> list[install.Task]:
    tasks = not_none(QgsApplication.taskManager()).activeTasks()
    return [task for task in tasks if isinstance(task, install.Task)]


@pytest.fixture(autouse=True)
def installs_end() -> c.Iterator[None]:
    """Stops the run when a cancelled install keeps running. Its thread
    stays, and every later test that waits for the tasks of QGIS would fail
    after its timeout, which takes longer than the jobs may run."""
    yield
    for task in installing():
        task.cancel()
    try:
        wait_for(lambda: not installing(), 30)
    except AssertionError:
        names = ', '.join(task.description() for task in installing())
        stacks = '\n'.join(
            ''.join(traceback.format_stack(frame))
            for ident, frame in sys._current_frames().items()
            if ident != threading.get_ident()
        )
        pytest.exit(
            f'Cancelled, but still running: {names}\n{stacks}', returncode=1
        )


@pytest.fixture(autouse=True)
def empty_project() -> c.Iterator[None]:
    """Starts and ends with an empty project."""
    QgsProject.instance().clear()
    yield
    QgsProject.instance().clear()


@pytest.fixture
def provider(qgis_processing: None) -> c.Iterator[Provider]:
    """The Processing provider, registered as the plugin does."""
    provider = Provider()
    registry = QgsApplication.processingRegistry()
    # The provider asks the tool for its rules in a task when it has not
    # been asked, so it is asked here and the algorithms are there at once
    with contextlib.suppress(ExecutableError):
        binary.rules()
    registry.addProvider(provider)
    yield provider
    # The plugin replaces a provider of the same id, so this one may be gone
    if registry.providerById(provider.id()) is provider:
        registry.removeProvider(provider)


@pytest.fixture
def configure() -> c.Callable[[str | Path], None]:
    def set_path(path: str | Path) -> None:
        settings = QgsSettings()
        settings.setValue(Setting.EXECUTABLE.value, str(path))
        # A task reads the setting on a thread of its own, which sees only
        # what is in the file
        settings.sync()

    return set_path


@pytest.fixture
def executable(configure: c.Callable[[str | Path], None]) -> str:
    """Configures the real executable, skipping without one."""
    path = real_executable()
    if path is None:
        pytest.skip('set TOPOLOGY_CHECKER to the topology-checker executable')
    configure(path)
    return path


@pytest.fixture
def dock(
    qgis_iface: QgisInterface, executable: str, provider: Provider
) -> c.Iterator[Dock]:
    """A dock of the real executable, as a user opens it."""
    dock = Dock(qgis_iface)
    dock.refresh()
    yield dock
    dock.cancel()
    wait_for(lambda: dock.task is None)
    dock.deleteLater()


@pytest.fixture
def bar(qgis_iface: QgisInterface, monkeypatch: pytest.MonkeyPatch) -> t.Any:
    """The message bar of the dock, which remembers its messages and the
    arguments of each `pushMessage`, as `bar.calls`."""
    bar = qgis_iface.messageBar()
    # The bar belongs to the whole session: what earlier tests put in it,
    # as strings and as widgets, is not for this one
    bar.clear_messages()
    bar.clearWidgets()
    bar.calls = []
    original = bar.pushMessage

    def spy(*args: t.Any, **kwargs: t.Any) -> None:
        bar.calls.append((args, kwargs))
        original(*args, **kwargs)

    monkeypatch.setattr(bar, 'pushMessage', spy)
    return bar


@pytest.fixture
def window(
    qgis_iface: QgisInterface, monkeypatch: pytest.MonkeyPatch
) -> c.Iterator[QMainWindow]:
    """A main window that holds the docks, as the one of QGIS does."""
    window = QMainWindow()
    monkeypatch.setattr(qgis_iface, 'mainWindow', lambda: window)
    monkeypatch.setattr(
        qgis_iface,
        'addDockWidget',
        lambda area, dock: window.addDockWidget(area, dock),
    )
    monkeypatch.setattr(
        qgis_iface,
        'removeDockWidget',
        lambda dock: window.removeDockWidget(dock),
    )
    yield window
    window.close()


@pytest.fixture
def bare(
    qgis_iface: QgisInterface, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> c.Iterator[Dock]:
    """A dock with no tool to be found, as after a fresh install."""
    monkeypatch.setenv('PATH', str(tmp_path / 'nothing'))
    dock = Dock(qgis_iface)
    dock.refresh()
    yield dock
    dock.deleteLater()
