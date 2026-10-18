"""The path of the tool is a setting of Processing too. Choosing the tool in
the dock must leave that setting with the chosen path, or the Options dialog,
which saves what Processing holds when it is closed with OK, puts back the
path that was there before."""

from __future__ import annotations

from pathlib import Path

import pytest
from processing.core.ProcessingConfig import ProcessingConfig
from qgis.core import QgsSettings
from qgis.PyQt.QtWidgets import QFileDialog

from topology_rules.core import binary
from topology_rules.enums import Setting
from topology_rules.panel.dock import Dock
from topology_rules.provider import SETTING_NAME

from .helpers import probe_executable


@pytest.fixture
def chosen(dock: Dock, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Another tool, chosen in the dock with the button that browses."""
    (tmp_path / 'tools').mkdir()
    path = probe_executable(tmp_path / 'tools', name='chosen-tool')
    monkeypatch.setattr(
        QFileDialog, 'getOpenFileName', lambda *args, **kwargs: (str(path), '')
    )
    return path


def press_ok_in_the_options() -> None:
    """What the Options dialog does on OK: every setting is set to what the
    dialog shows, which it filled in when it opened, and saved."""
    settings = QgsSettings()
    for setting in list(ProcessingConfig.settings.values()):
        setting.setValue(setting.value)
        setting.save(settings)
    settings.sync()


def test_processing_has_the_path_that_was_chosen(dock: Dock, chosen: Path):
    dock.choose_executable()
    assert ProcessingConfig.getSetting(SETTING_NAME) == str(chosen)


def test_the_setting_of_the_plugin_has_the_path_that_was_chosen(
    dock: Dock, chosen: Path
):
    dock.choose_executable()
    assert QgsSettings().value(Setting.EXECUTABLE.value) == str(chosen)


def test_the_options_dialog_keeps_the_path_that_was_chosen(
    dock: Dock, chosen: Path
):
    dock.choose_executable()
    press_ok_in_the_options()
    assert QgsSettings().value(Setting.EXECUTABLE.value) == str(chosen)


def test_the_tool_is_still_the_one_that_was_chosen_after_the_options_dialog(
    dock: Dock, chosen: Path
):
    dock.choose_executable()
    press_ok_in_the_options()
    assert binary.find_executable() == chosen
