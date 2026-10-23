"""The engine in the panel: the gear menu, the install of the Python package
from PyPI and the setup page, against a PyPI of the tests on 127.0.0.1.

QGIS is told whether it can use the package, whatever is installed here, so
that the tests run the same with the package and without it."""

from __future__ import annotations

import collections.abc as c
import html
import importlib
import platform
import sys
import threading
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsSettings
from qgis.gui import QgisInterface
from qgis.PyQt.QtCore import Qt, QUrl
from qgis.PyQt.QtGui import QDesktopServices
from qgis.PyQt.QtWidgets import (
    QApplication,
    QFileDialog,
    QLayout,
    QStyleFactory,
    QToolButton,
    QWidget,
)

from topology_rules.core import binary, engine, folders, install, wheels
from topology_rules.core.binary import Rule
from topology_rules.core.engine import Engine, Package
from topology_rules.enums import Setting
from topology_rules.exceptions import PackageError
from topology_rules.panel import installer, startup
from topology_rules.panel.dock import Dock
from topology_rules.panel.installer import GUIDE, InstallDialog
from topology_rules.panel.menu import PanelMenu
from topology_rules.provider import Provider
from topology_rules.utils import not_none

from .helpers import choose_engine, message_text, real_executable, wait_for
from .test_install import Index, load_from, trickle, version_folder
from .wheel_helpers import (
    LINUX,
    VERSION,
    WHEELS,
    answer,
    make_wheel,
    pretend_linux,
    release_file,
    sha256,
    wheel_name,
)

MISSING = (
    'The topology-checker Python package cannot be imported: No module '
    "named 'topology_checker'"
)
JSON = f'/pypi/topology-checker/{VERSION}/json'


# Fixtures


@pytest.fixture(autouse=True)
def linux(monkeypatch: pytest.MonkeyPatch) -> None:
    """A QGIS that can use the package: Linux x86_64, Python 3.12.8."""
    pretend_linux(monkeypatch)


@pytest.fixture(autouse=True)
def home(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    """The folder of the package, in a profile of the test."""
    folder = tmp_path / 'profile' / 'python' / 'dependencies' / 'topology_rules'
    monkeypatch.setattr(folders, 'folder', lambda: folder)
    return folder


def package_modules() -> list[str]:
    return [
        name
        for name in sys.modules
        if name == wheels.MODULE or name.startswith(f'{wheels.MODULE}.')
    ]


@pytest.fixture(autouse=True)
def imports(monkeypatch: pytest.MonkeyPatch) -> c.Iterator[None]:
    """A sys.path of the test, and no topology_checker imported; what was
    imported before comes back after."""
    monkeypatch.setattr(sys, 'path', list(sys.path))
    for name in package_modules():
        monkeypatch.delitem(sys.modules, name)
    yield
    for name in package_modules():
        del sys.modules[name]
    importlib.invalidate_caches()


@pytest.fixture(autouse=True)
def no_package(monkeypatch: pytest.MonkeyPatch) -> None:
    """QGIS cannot import the package."""

    def missing() -> Package:
        raise PackageError(MISSING)

    monkeypatch.setattr(engine, 'find_package', missing)


def executable_rules() -> tuple[Rule, ...]:
    path = real_executable()
    if path is None:
        pytest.skip('set TOPOLOGY_CHECKER to the topology-checker executable')
    return binary.probe(path, Path(path).stat().st_mtime_ns)


def make_usable(monkeypatch: pytest.MonkeyPatch, home: Path) -> Package:
    """QGIS can use the package, which knows the rules of the executable."""
    found = Package(VERSION, home / VERSION / 'topology_checker')
    monkeypatch.setattr(engine, 'find_package', lambda: found)
    monkeypatch.setattr(engine, 'rules', lambda used: executable_rules())
    return found


@pytest.fixture
def usable(monkeypatch: pytest.MonkeyPatch, home: Path) -> Package:
    return make_usable(monkeypatch, home)


@pytest.fixture
def old_python(monkeypatch: pytest.MonkeyPatch) -> str:
    """A QGIS whose Python is too old for the package, and why."""
    monkeypatch.setattr(platform, 'python_version', lambda: '3.9.18')
    reason = wheels.unsupported()
    assert reason is not None
    return reason


@pytest.fixture(autouse=True)
def tasks_end() -> c.Iterator[None]:
    """Closes the dialogs that a test left, and stops their tasks before the
    folder and the PyPI of the test go."""
    yield
    for widget in QApplication.topLevelWidgets():
        if isinstance(widget, InstallDialog):
            widget.stop()
            widget.hide()
            widget.deleteLater()
    for task in list(installer.running):
        task.cancel()
    wait_for(lambda: not installer.running, 30)


@pytest.fixture
def pypi(monkeypatch: pytest.MonkeyPatch) -> c.Iterator[Index]:
    index = Index()
    thread = threading.Thread(target=index.serve_forever, daemon=True)
    thread.start()
    monkeypatch.setattr(install, 'PYPI', f'{index.url}/pypi')
    yield index
    for task in list(installer.running):
        task.cancel()
    wait_for(lambda: not installer.running, 30)
    index.shutdown()
    index.server_close()


@pytest.fixture
def published(pypi: Index) -> tuple[Index, bytes]:
    """The release of the plugin's version, with a wheel for each platform
    of which only the Linux one is served."""
    content = make_wheel()
    files = []
    for kind, tag in WHEELS.items():
        body = content if kind == 'linux' else make_wheel(tag=tag)
        url = pypi.serve(f'/files/{wheel_name(tag)}', body, kind='x')
        files.append(release_file(tag, body, url))
    pypi.release(answer(files))
    return pypi, content


@pytest.fixture
def refreshes(monkeypatch: pytest.MonkeyPatch, provider: Provider) -> list[str]:
    """What was refreshed: the toolbox, then the panel, which does not look
    for its rules here."""
    calls: list[str] = []
    provider.algorithmsLoaded.connect(lambda: calls.append('toolbox'))
    monkeypatch.setattr(startup, 'refresh', lambda dock: calls.append('panel'))
    return calls


@pytest.fixture
def opened(monkeypatch: pytest.MonkeyPatch) -> list[QUrl]:
    """The URLs that QGIS was asked to open."""
    urls: list[QUrl] = []
    monkeypatch.setattr(
        QDesktopServices, 'openUrl', lambda url: bool(urls.append(url)) or True
    )
    return urls


def texts(menu: PanelMenu) -> list[str]:
    """The items that the menu shows, a line for a separator."""
    return [
        action.text() or '---'
        for action in menu.actions()
        if action.isVisible()
    ]


def looked_up(dialog: InstallDialog) -> InstallDialog:
    wait_for(lambda: dialog.task is None, 30)
    return dialog


def open_dialog(update: bool = False) -> InstallDialog:
    dialog = InstallDialog(update=update)
    dialog.show()
    return dialog


def install_in(dialog: InstallDialog) -> InstallDialog:
    looked_up(dialog)
    dialog.install_button.click()
    return looked_up(dialog)


def shown(widget: t.Any) -> bool:
    """Whether the widget shows when its window does."""
    return not widget.isHidden()


def row_of(layout: QLayout, widget: QWidget) -> list[QWidget | None]:
    """The widgets of the layout, or of a layout in it, with the widget."""
    items = [not_none(layout.itemAt(i)) for i in range(layout.count())]
    widgets = [item.widget() for item in items]
    if widget in widgets:
        return widgets
    for item in items:
        inner = item.layout()
        if inner is not None and (found := row_of(inner, widget)):
            return found
    return []


# The menu


def test_the_menu_has_the_engines_then_the_installs():
    menu = PanelMenu()
    menu.fill()
    assert texts(menu) == [
        'Engine',
        'Automatic',
        'Python package',
        'Executable',
        '---',
        'Install Python package…',
        'Choose executable…',
    ]


def test_the_engines_have_a_section_where_the_style_draws_one(
    monkeypatch: pytest.MonkeyPatch,
):
    style = QStyleFactory.create('Fusion')
    monkeypatch.setattr(PanelMenu, 'style', lambda self: style)
    menu = PanelMenu()
    heading = menu.actions()[0]
    assert heading.isSeparator()
    assert heading.text() == 'Engine'


def test_the_engines_have_a_title_where_the_style_draws_no_sections(
    monkeypatch: pytest.MonkeyPatch,
):
    style = QStyleFactory.create('Windows')
    monkeypatch.setattr(PanelMenu, 'style', lambda self: style)
    menu = PanelMenu()
    heading = menu.actions()[0]
    assert not heading.isSeparator()
    assert heading.text() == 'Engine'
    assert not heading.isEnabled()


@pytest.mark.parametrize('chosen', list(Engine), ids=[e.value for e in Engine])
def test_the_chosen_engine_is_checked(chosen: Engine):
    choose_engine(chosen)
    menu = PanelMenu()
    menu.fill()
    assert [action.isChecked() for action in menu.engines.values()] == [
        choice is chosen for choice in Engine
    ]
    assert menu.group.isExclusive()


def test_the_package_cannot_be_chosen_where_qgis_cannot_use_it():
    menu = PanelMenu()
    menu.fill()
    package = menu.engines[Engine.PACKAGE]
    assert not package.isEnabled()
    assert package.toolTip() == MISSING
    assert package.statusTip() == MISSING


def test_a_package_that_qgis_can_use_can_be_chosen(usable: Package):
    menu = PanelMenu()
    menu.fill()
    package = menu.engines[Engine.PACKAGE]
    assert package.isEnabled()
    assert package.toolTip() == (
        f'topology-checker {VERSION} in {usable.location}'
    )


def test_the_executable_cannot_be_chosen_without_one(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    monkeypatch.setenv('PATH', str(tmp_path / 'nothing'))
    menu = PanelMenu()
    menu.fill()
    action = menu.engines[Engine.EXECUTABLE]
    assert not action.isEnabled()
    assert action.toolTip().startswith('topology-checker was not found.')


def test_the_executable_can_be_chosen_with_one(executable: str):
    menu = PanelMenu()
    menu.fill()
    action = menu.engines[Engine.EXECUTABLE]
    assert action.isEnabled()
    assert action.toolTip() == str(Path(executable))


def test_automatic_is_always_there_to_choose(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    monkeypatch.setenv('PATH', str(tmp_path / 'nothing'))
    menu = PanelMenu()
    menu.fill()
    assert menu.engines[Engine.AUTOMATIC].isEnabled()
    assert 'else the executable' in menu.engines[Engine.AUTOMATIC].toolTip()


def test_the_install_says_why_this_qgis_cannot_use_the_package(
    old_python: str,
):
    menu = PanelMenu()
    menu.fill()
    assert menu.install_action.isVisible()
    assert not menu.install_action.isEnabled()
    assert menu.install_action.toolTip() == old_python


def test_an_installed_package_can_be_removed_and_not_installed(home: Path):
    version_folder(home, VERSION)
    menu = PanelMenu()
    menu.fill()
    assert texts(menu)[-2:] == ['Remove Python package', 'Choose executable…']
    assert not menu.update_action.isVisible()
    assert not menu.install_action.isVisible()


def test_an_older_package_can_be_updated(home: Path):
    version_folder(home, '0.0.1')
    menu = PanelMenu()
    menu.fill()
    assert texts(menu)[-3:] == [
        f'Update Python package to {VERSION}…',
        'Remove Python package',
        'Choose executable…',
    ]


def test_a_package_that_qgis_has_is_not_installed_or_removed(
    usable: Package,
):
    menu = PanelMenu()
    menu.fill()
    assert texts(menu)[-2:] == ['---', 'Choose executable…']


def test_the_menu_is_filled_as_it_opens(home: Path):
    menu = PanelMenu()
    menu.fill()
    assert menu.install_action.isVisible()
    version_folder(home, VERSION)
    menu.aboutToShow.emit()
    assert not menu.install_action.isVisible()


# The gear button


def test_the_gear_is_next_to_help(bare: Dock):
    button = bare.menu_button
    assert isinstance(button, QToolButton)
    assert button.menu() is bare.menu
    assert button.popupMode() == QToolButton.ToolButtonPopupMode.InstantPopup
    assert button.autoRaise()
    assert button.toolTip() == 'Settings'
    body = not_none(bare.widget())
    row = row_of(not_none(body.layout()), bare.help_button)
    assert row[-2:] == [button, bare.help_button]


def test_the_gear_shows_on_the_setup_page(bare: Dock):
    assert not bare.setup_page.isHidden()
    assert shown(bare.menu_button)


# Choices and refreshes


def test_choosing_an_engine_saves_it_and_refreshes(
    dock: Dock, refreshes: list[str]
):
    dock.menu.fill()
    dock.menu.engines[Engine.AUTOMATIC].trigger()
    assert engine.chosen() is Engine.AUTOMATIC
    assert refreshes == ['toolbox', 'panel']


def test_choosing_the_engine_that_is_chosen_changes_nothing(
    dock: Dock, refreshes: list[str]
):
    dock.menu.fill()
    dock.menu.engines[Engine.EXECUTABLE].trigger()
    assert refreshes == []


def test_the_package_runs_the_checks_once_chosen(dock: Dock, usable: Package):
    dock.menu.fill()
    dock.menu.engines[Engine.PACKAGE].trigger()
    assert engine.chosen() is Engine.PACKAGE
    assert engine.resolved() is Engine.PACKAGE
    assert dock.available
    assert dock.form.rules == executable_rules()


def test_the_executable_can_be_chosen_from_the_menu(
    bare: Dock, executable: str, monkeypatch: pytest.MonkeyPatch
):
    QgsSettings().remove(Setting.EXECUTABLE.value)
    monkeypatch.setattr(
        QFileDialog,
        'getOpenFileName',
        lambda *args, **kwargs: (executable, ''),
    )
    bare.menu.choose_action.trigger()
    wait_for(lambda: bare.probing is None)
    assert QgsSettings().value(Setting.EXECUTABLE.value) == executable
    assert bare.setup_page.isHidden()


def test_remove_deletes_the_package_and_refreshes(
    dock: Dock, home: Path, bar: t.Any, refreshes: list[str]
):
    version_folder(home, VERSION)
    dock.menu.fill()
    dock.menu.remove_action.trigger()
    assert not home.exists()
    assert install.installed() is None
    assert 'Removed the Python package.' in message_text(bar)
    assert 'Restart' not in message_text(bar)
    assert refreshes == ['toolbox', 'panel']


def test_removing_a_loaded_package_asks_for_a_restart(
    dock: Dock, home: Path, bar: t.Any, monkeypatch: pytest.MonkeyPatch
):
    load_from(version_folder(home, VERSION), monkeypatch)
    dock.menu.fill()
    dock.menu.remove_action.trigger()
    assert (
        'Removed the Python package. Restart QGIS to unload it.'
        in message_text(bar)
    )


def test_removing_the_chosen_package_goes_back_to_automatic(
    dock: Dock, home: Path, bar: t.Any, refreshes: list[str]
):
    version_folder(home, VERSION)
    choose_engine(Engine.PACKAGE)
    dock.menu.fill()
    dock.menu.remove_action.trigger()
    assert engine.chosen() is Engine.AUTOMATIC


def test_one_install_dialog_opens_at_a_time(dock: Dock, pypi: Index):
    dock.menu.fill()
    dock.menu.install_action.trigger()
    first = dock.installer
    dock.menu.install_action.trigger()
    assert first is not None
    assert dock.installer is first
    assert first.isVisible()
    assert first.windowTitle() == 'Install Python package'


def test_an_install_refreshes_the_toolbox_and_the_panel(
    dock: Dock, published: tuple[Index, bytes], refreshes: list[str]
):
    dock.menu.fill()
    dock.menu.install_action.trigger()
    install_in(not_none_dialog(dock))
    assert install.installed() == VERSION
    assert refreshes == ['toolbox', 'panel']


def test_update_opens_the_update(dock: Dock, home: Path, pypi: Index):
    version_folder(home, '0.0.1')
    dock.menu.fill()
    dock.menu.update_action.trigger()
    dialog = not_none_dialog(dock)
    assert dialog.windowTitle() == 'Update Python package'
    assert dialog.install_button.text() == 'Update'


def not_none_dialog(dock: Dock) -> InstallDialog:
    assert dock.installer is not None
    return dock.installer


# The install dialog


def test_the_dialog_looks_up_the_release_first(pypi: Index):
    pypi.routes[JSON] = trickle
    dialog = open_dialog()
    assert dialog.status.text() == f'Looking up topology-checker {VERSION}…'
    assert not dialog.install_button.isEnabled()
    assert dialog.facts.isHidden()
    assert dialog.cancel_button.text() == 'Cancel'
    wait_for(lambda: bool(pypi.asked), 30)


def test_cancel_while_looking_up_closes(pypi: Index):
    pypi.routes[JSON] = trickle
    dialog = open_dialog()
    wait_for(lambda: bool(pypi.asked), 30)
    dialog.cancel_button.click()
    assert not dialog.isVisible()
    wait_for(lambda: not installer.running, 30)


def test_the_dialog_says_what_it_would_install(
    published: tuple[Index, bytes], home: Path
):
    pypi, content = published
    dialog = looked_up(open_dialog())
    name = wheel_name(LINUX)
    assert dialog.status.isHidden()
    assert shown(dialog.facts)
    assert dialog.package_label.text() == f'topology-checker {VERSION}'
    assert dialog.file_field.text() == (
        f'{name} ({install.size_text(len(content))})'
    )
    assert dialog.source_field.text() == f'{pypi.url}/files/{name}'
    assert dialog.sha256_field.text() == sha256(content)
    assert dialog.folder_field.text() == str(home / VERSION)
    assert dialog.dependencies_label.text() == 'None'
    assert dialog.install_button.isEnabled()
    assert dialog.install_button.text() == 'Install'
    # Nothing is downloaded before the install is asked for
    assert pypi.asked == [JSON]
    assert not (home / VERSION).exists()


def test_the_command_does_the_same_by_hand(published: tuple[Index, bytes]):
    dialog = looked_up(open_dialog())
    assert dialog.command_box.title() == 'Command for Terminal'
    assert dialog.command_box.isCollapsed()
    assert dialog.command.toPlainText() == install.command(VERSION)
    assert '--no-deps' in dialog.command.toPlainText()
    dialog.copy_command.click()
    clipboard = QApplication.clipboard()
    assert clipboard is not None
    assert clipboard.text() == install.command(VERSION)


def test_open_shows_the_folder_of_the_package(
    published: tuple[Index, bytes], home: Path, opened: list[QUrl]
):
    dialog = looked_up(open_dialog())
    dialog.open_button.click()
    # The version folder comes with the install; the log is there already
    assert [Path(url.toLocalFile()) for url in opened] == [home]


def test_a_release_that_pypi_does_not_have_fails_with_the_command(
    pypi: Index,
):
    dialog = looked_up(open_dialog())
    text = dialog.status.text()
    assert text.startswith(f'PyPI has no topology-checker {VERSION}<br>')
    assert f'href="{GUIDE}"' in text
    assert dialog.status.openExternalLinks()
    assert shown(dialog.command_box)
    assert not dialog.command_box.isCollapsed()
    assert dialog.install_button.isHidden()
    assert dialog.facts.isHidden()
    assert dialog.cancel_button.text() == 'Close'


def test_a_qgis_that_cannot_use_the_package_shows_why_and_the_command(
    pypi: Index, old_python: str
):
    dialog = looked_up(open_dialog())
    assert dialog.status.text().startswith(old_python)
    assert not dialog.command_box.isCollapsed()
    assert dialog.install_button.isHidden()
    assert pypi.asked == []


def test_the_install_shows_its_lines_as_it_goes(
    published: tuple[Index, bytes],
):
    dialog = looked_up(open_dialog())
    dialog.install_button.click()
    assert dialog.status.text() == f'Installing topology-checker {VERSION}…'
    assert dialog.facts.isHidden()
    assert dialog.command_box.isHidden()
    assert dialog.install_button.isHidden()
    assert shown(dialog.log)
    assert shown(dialog.copy_log)
    assert dialog.cancel_button.isEnabled()
    assert dialog.cancel_button.text() == 'Cancel'
    looked_up(dialog)
    lines = dialog.log.toPlainText().splitlines()
    # The lines of the look-up come first
    assert lines[0].startswith('Python 3.12.8')
    assert any(line.startswith('Downloading ') for line in lines)
    assert any(line.endswith(' matches PyPI') for line in lines)
    assert lines[-1] == f'Loaded topology-checker {VERSION}'
    dialog.copy_log.click()
    clipboard = QApplication.clipboard()
    assert clipboard is not None
    assert clipboard.text() == dialog.log.toPlainText()


def test_a_finished_install_says_it_is_in_use(
    published: tuple[Index, bytes],
    monkeypatch: pytest.MonkeyPatch,
    home: Path,
):
    dialog = looked_up(open_dialog())
    done: list[str] = []
    dialog.installed.connect(done.append)
    # The package of the tests is not one that QGIS can use
    make_usable(monkeypatch, home)
    install_in(dialog)
    assert dialog.status.text() == (
        f'Installed topology-checker {VERSION}. In use now.'
    )
    assert done == [VERSION]
    assert dialog.cancel_button.text() == 'Close'
    assert dialog.cancel_button.isEnabled()
    dialog.cancel_button.click()
    assert not dialog.isVisible()


def test_an_install_that_qgis_cannot_use_says_why(
    published: tuple[Index, bytes],
):
    dialog = install_in(open_dialog())
    assert dialog.status.text() == html.escape(
        f'Installed topology-checker {VERSION}. {MISSING}'
    )


def test_an_install_over_a_loaded_package_needs_a_restart(
    published: tuple[Index, bytes],
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
):
    load_from(version_folder(home, '0.0.1'), monkeypatch)
    dialog = install_in(open_dialog(update=True))
    assert dialog.status.text() == (
        f'Installed topology-checker {VERSION}. Restart QGIS to use it.'
    )


def test_installing_uses_the_package_instead_of_the_chosen_executable(
    published: tuple[Index, bytes],
):
    install_in(open_dialog())
    assert engine.chosen() is Engine.AUTOMATIC


def test_installing_keeps_the_package_chosen(published: tuple[Index, bytes]):
    choose_engine(Engine.PACKAGE)
    install_in(open_dialog())
    assert engine.chosen() is Engine.PACKAGE


def test_a_failed_install_shows_the_error_and_the_command(
    pypi: Index, home: Path
):
    content = make_wheel()
    pypi.release(answer([release_file(LINUX, content, f'{pypi.url}/gone')]))
    dialog = install_in(open_dialog())
    assert dialog.status.text().startswith(
        f'Cannot fetch {pypi.url}/gone: HTTP 404 Not Found<br>'
    )
    assert shown(dialog.log)
    assert shown(dialog.command_box)
    assert not dialog.command_box.isCollapsed()
    assert dialog.cancel_button.text() == 'Close'
    assert install.installed() is None


def test_cancel_stops_the_install_and_says_so(pypi: Index, home: Path):
    url = pypi.url + '/files/slow.whl'
    pypi.routes['/files/slow.whl'] = trickle
    slow = {
        'filename': wheel_name(LINUX),
        'url': url,
        'size': 10_000_000,
        'digests': {'sha256': sha256(b'')},
        'packagetype': 'bdist_wheel',
    }
    pypi.release(answer([slow]))
    dialog = looked_up(open_dialog())
    dialog.install_button.click()
    task = dialog.task
    assert task is not None
    wait_for(lambda: task.progress() > 0, 30)
    dialog.cancel_button.click()
    # It stays to say when the install stopped
    assert dialog.isVisible()
    assert dialog.status.text() == 'Cancelling…'
    assert not dialog.cancel_button.isEnabled()
    looked_up(dialog)
    assert dialog.status.text().startswith('Cancelled<br>')
    assert dialog.cancel_button.isEnabled()
    assert dialog.cancel_button.text() == 'Close'
    assert not (home / VERSION).exists()
    dialog.reject()
    assert not dialog.isVisible()


def test_closing_the_panel_stops_the_install(
    qgis_iface: QgisInterface, pypi: Index
):
    pypi.routes[JSON] = trickle
    dock = Dock(qgis_iface)
    dock.install_package()
    wait_for(lambda: bool(pypi.asked), 30)
    dock.dispose()
    assert dock.installer is None
    wait_for(lambda: not installer.running, 30)
    dock.deleteLater()


# The setup page


def test_the_setup_page_leads_with_the_install(bare: Dock):
    page = bare.setup_page
    assert shown(page.install_button)
    assert page.install_button.text() == 'Install Python package…'
    assert page.install_button.isDefault()
    assert page.browse_button.text() == 'Choose executable…'
    order = row_of(not_none(page.layout()), page.status)
    assert order[:4] == [
        page.status,
        page.install_button,
        page.browse_button,
        page.guide,
    ]


def test_the_setup_page_has_no_install_where_qgis_cannot_use_it(
    bare: Dock, old_python: str
):
    bare.refresh()
    assert bare.setup_page.install_button.isHidden()
    assert shown(bare.setup_page.browse_button)


def test_the_setup_page_says_why_the_engine_cannot_run(bare: Dock):
    assert bare.setup_page.status.text().startswith(
        'topology-checker was not found.'
    )


def test_the_setup_page_shows_a_reason_as_it_is(bare: Dock):
    reason = "Can't run <topology-checker> & co"
    bare.setup_page.explain(reason)
    assert bare.setup_page.status.text() == reason
    assert bare.setup_page.status.textFormat() == Qt.TextFormat.PlainText


def test_install_on_the_setup_page_opens_the_dialog(bare: Dock, pypi: Index):
    bare.install_button.click()
    assert bare.installer is not None
    assert bare.installer.isVisible()


def test_an_install_on_the_setup_page_shows_the_rules(
    bare: Dock,
    published: tuple[Index, bytes],
    monkeypatch: pytest.MonkeyPatch,
    home: Path,
    provider: Provider,
):
    executable_rules()
    bare.install_button.click()
    dialog = looked_up(not_none_dialog(bare))
    # The package of the tests is not one that QGIS can use
    make_usable(monkeypatch, home)
    install_in(dialog)
    assert dialog.status.text().endswith('In use now.')
    assert bare.setup_page.isHidden()
    assert bare.available
    assert bare.form.rules == executable_rules()
    assert provider.algorithms()
