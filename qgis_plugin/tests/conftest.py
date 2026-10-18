"""Fixtures of the tests: the executable, the provider, the dock, the
message bar, and a clean project and settings for each test."""

from __future__ import annotations

import atexit
import collections.abc as c
import contextlib
import shutil
import sys
import tempfile
import traceback
import types
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsApplication, QgsProject, QgsSettings
from qgis.gui import QgisInterface
from qgis.PyQt.QtCore import QSettings
from qgis.PyQt.QtWidgets import QApplication, QMainWindow

from topology_rules.core import binary
from topology_rules.enums import Setting
from topology_rules.exceptions import ExecutableError
from topology_rules.panel.dock import Dock
from topology_rules.panel.rules_page import COLLAPSED
from topology_rules.provider import Provider

from .helpers import real_executable, wait_for


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


@pytest.fixture(autouse=True, scope='session')
def ui_font(qgis_app: QgsApplication) -> None:
    """Gives the widgets the size of text that a user has. Without a window
    system a platform picks a font of its own: Windows without a screen has
    Helvetica at 12 points and 100 dpi, a third larger than anything that a
    person uses, which makes a panel wider than it is for them. A size in
    pixels does not depend on the dpi, so what the layout tests measure is the
    same on Linux, macOS and Windows. The family is the one of the system. The
    tests of a large font set their own."""
    font = QApplication.font()
    font.setPixelSize(UI_TEXT_PIXELS)
    QApplication.setFont(font)


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
    probe.cache_clear()
    with probe.lock:
        probe.done.update(rules)
        probe.readable.update(formats)


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
