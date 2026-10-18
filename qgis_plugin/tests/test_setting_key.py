"""The key under which the path of the tool is stored. Users have set it
already, in Settings > Options > Processing > Providers > Topology Rules, and
it has to stay as it is, so it is written out in full here and not built from
the names that the plugin has for it."""

from __future__ import annotations

from pathlib import Path

from processing.core.ProcessingConfig import ProcessingConfig
from qgis.core import QgsSettings

from topology_rules.core import binary
from topology_rules.enums import Setting
from topology_rules.provider import SETTING_NAME, Provider

# What QGIS stores for a setting of Processing: its prefix, and its name
KEY = 'Processing/Configuration/TOPOLOGY_RULES_EXECUTABLE'
NAME = 'TOPOLOGY_RULES_EXECUTABLE'


def test_the_path_of_the_tool_is_stored_under_the_key_that_users_have():
    assert Setting.EXECUTABLE.value == KEY


def test_the_setting_of_processing_has_the_name_at_the_end_of_the_key():
    assert SETTING_NAME == NAME


def test_the_setting_of_processing_is_stored_under_the_key(provider: Provider):
    assert ProcessingConfig.settings[SETTING_NAME].qname == KEY


def test_a_path_stored_under_the_key_is_the_setting_of_processing(
    provider: Provider, tmp_path: Path
):
    QgsSettings().setValue(KEY, str(tmp_path / 'topology-checker'))
    ProcessingConfig.readSettings()
    assert ProcessingConfig.getSetting(SETTING_NAME) == str(
        tmp_path / 'topology-checker'
    )


def test_the_setting_of_processing_is_stored_under_the_key_when_it_is_set(
    provider: Provider, tmp_path: Path
):
    # The setting of a file is only taken for a file that is there
    file = tmp_path / 'topology-checker'
    file.write_text('')
    ProcessingConfig.setSettingValue(SETTING_NAME, str(file))
    assert QgsSettings().value(KEY) == str(file)


def test_the_plugin_reads_the_path_stored_under_the_key(tmp_path: Path):
    path = str(tmp_path / 'topology-checker')
    QgsSettings().setValue(KEY, path)
    assert binary.configured_path() == path
