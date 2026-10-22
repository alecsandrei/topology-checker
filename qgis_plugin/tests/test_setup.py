"""The dock before the tool is found: a page that says how to get it."""

from __future__ import annotations

from pathlib import Path

import pytest
from qgis.core import QgsSettings
from qgis.PyQt.QtWidgets import QFileDialog, QLabel

from topology_rules.enums import Setting
from topology_rules.panel.dock import Dock
from topology_rules.panel.setup import INSTALL, PYPI, RELEASES, SetupPage

from .helpers import message_text, script_executable, wait_for


def choose_file(monkeypatch: pytest.MonkeyPatch, path: object) -> None:
    monkeypatch.setattr(
        QFileDialog,
        'getOpenFileName',
        lambda *args, **kwargs: (str(path), ''),
    )


def test_without_the_tool_the_setup_page_is_shown(bare: Dock):
    assert not bare.setup_page.isHidden()


def test_without_the_tool_the_tabs_are_hidden(bare: Dock):
    assert bare.tabs.isHidden()


def test_the_setup_page_links_to_the_install_page(bare: Dock):
    links = [
        label
        for label in bare.setup_page.findChildren(QLabel)
        if f'href="{INSTALL}"' in label.text()
    ]
    assert len(links) == 1
    assert links[0].openExternalLinks()


def test_the_setup_page_presents_pypi_and_releases(bare: Dock):
    text = bare.setup_page.status.text()
    assert f'href="{PYPI}"' in text
    assert f'href="{RELEASES}"' in text
    assert 'pip install topology-checker' in text
    assert 'For maximum performance' in text
    assert not bare.setup_page.browse_button.isHidden()


def test_setup_page_escapes_reason(bare: Dock):
    bare.setup_page.explain('<script>alert("test")&foo</script>')
    text = bare.setup_page.status.text()
    assert '<script>' not in text
    assert '&lt;script&gt;' in text
    assert '&amp;foo' in text


def test_setup_page_explain_instructions():
    page = SetupPage()
    page.checking()
    assert page.browse_button.isHidden()

    reason = 'Tool not found: <check> & "verify"'
    page.explain(reason)

    assert not page.browse_button.isHidden()
    assert page.status.openExternalLinks()

    text = page.status.text()

    # 1. Reason is HTML escaped
    assert '&lt;check&gt; &amp; &quot;verify&quot;' in text
    assert '<check>' not in text

    # 2. PyPI instructions with link and command
    assert f'href="{PYPI}"' in text
    assert '>PyPI</a>' in text
    assert 'pip install topology-checker' in text
    assert 'For maximum performance' in text

    # 3. GitHub Releases standalone CLI download link
    assert f'href="{RELEASES}"' in text
    assert '>GitHub Releases</a>' in text
    assert 'standalone CLI executable' in text

    # 4. Documentation install link
    assert f'href="{INSTALL}"' in text
    assert 'How to install topology-checker' in text


def test_with_the_tool_the_tabs_are_shown(dock: Dock):
    assert dock.setup_page.isHidden()
    assert not dock.tabs.isHidden()


def test_browsing_for_the_tool_saves_its_path(
    bare: Dock, executable: str, monkeypatch: pytest.MonkeyPatch
):
    QgsSettings().remove(Setting.EXECUTABLE.value)
    choose_file(monkeypatch, executable)
    bare.browse_button.click()
    wait_for(lambda: bare.probing is None)
    assert QgsSettings().value(Setting.EXECUTABLE.value) == executable


def test_browsing_for_the_tool_shows_the_form(
    bare: Dock, executable: str, monkeypatch: pytest.MonkeyPatch
):
    QgsSettings().remove(Setting.EXECUTABLE.value)
    choose_file(monkeypatch, executable)
    bare.browse_button.click()
    wait_for(lambda: bare.probing is None)
    assert bare.setup_page.isHidden()
    assert not bare.tabs.isHidden()
    assert bare.rule_box.count() >= 0


def test_a_canceled_browse_changes_nothing(
    bare: Dock, monkeypatch: pytest.MonkeyPatch
):
    choose_file(monkeypatch, '')
    bare.browse_button.click()
    assert not bare.setup_page.isHidden()
    assert not QgsSettings().value(Setting.EXECUTABLE.value, '')


def test_a_file_that_is_not_the_tool_keeps_the_page(
    bare: Dock, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, bar: object
):
    wrong = script_executable(tmp_path, 'sys.exit(1)\n', rules=[])
    choose_file(monkeypatch, wrong)
    bare.browse_button.click()
    wait_for(lambda: bare.probing is None)
    assert not bare.setup_page.isHidden()
    assert bare.tabs.isHidden()
    assert message_text(bar) or bare.setup_page.findChildren(QLabel)
