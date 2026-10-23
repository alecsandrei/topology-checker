"""The look of the buttons of the panel: an icon and a tip for each, and a
button that opens the documentation."""

from __future__ import annotations

import pytest
from qgis.PyQt.QtCore import QUrl
from qgis.PyQt.QtGui import QDesktopServices
from qgis.PyQt.QtWidgets import QApplication, QPushButton, QToolButton

from topology_rules.panel.dock import Dock

DOCS = 'https://alecsandrei.github.io/topology-checker/'
BUTTONS = [
    'add_button',
    'remove_button',
    'clear_button',
    'import_button',
    'export_button',
    'run_button',
]


@pytest.fixture
def opened(monkeypatch: pytest.MonkeyPatch) -> list[str]:
    urls: list[str] = []

    def record(url: QUrl) -> bool:
        urls.append(url.toString())
        return True

    monkeypatch.setattr(QDesktopServices, 'openUrl', record)
    return urls


# Icons


@pytest.mark.parametrize('name', BUTTONS)
def test_a_button_has_an_icon(dock: Dock, name: str):
    assert not getattr(dock, name).icon().isNull()


@pytest.mark.parametrize('name', BUTTONS)
def test_a_button_has_a_tip(dock: Dock, name: str):
    assert getattr(dock, name).toolTip()


def test_the_run_button_keeps_its_icon_while_it_cancels(dock: Dock):
    assert not dock.run_button.icon().isNull()


@pytest.mark.parametrize('name', BUTTONS)
def test_a_button_keeps_its_text(dock: Dock, name: str):
    assert getattr(dock, name).text()


@pytest.mark.usefixtures('real_font')
@pytest.mark.parametrize('name', ['import_button', 'export_button'])
def test_the_file_buttons_fit_the_narrowest_dock(dock: Dock, name: str):
    dock.show()
    dock.resize(dock.minimumSizeHint().width(), 900)
    QApplication.processEvents()
    button = getattr(dock, name)
    assert button.width() >= button.sizeHint().width()


# Help


def test_there_is_a_help_button(dock: Dock):
    assert isinstance(dock.help_button, QPushButton | QToolButton)
    assert dock.help_button.toolTip()


def test_the_help_button_has_an_icon(dock: Dock):
    assert not dock.help_button.icon().isNull()


def test_the_help_button_opens_the_documentation(dock: Dock, opened: list[str]):
    dock.help_button.click()
    assert len(opened) == 1
    assert opened[0].startswith(DOCS)


def test_the_help_button_is_there_without_the_tool(
    bare: Dock, opened: list[str]
):
    bare.help_button.click()
    assert len(opened) == 1
