"""Loading, reloading and unloading the plugin, as the Plugin Manager and
the Plugin Reloader do."""

from __future__ import annotations

import collections.abc as c
from pathlib import Path

import pytest
from qgis.core import QgsApplication, QgsProject
from qgis.gui import QgisInterface
from qgis.PyQt.QtWidgets import QDockWidget, QMainWindow

from topology_rules.enums import PROVIDER_ID
from topology_rules.plugin import TopologyRules

from .helpers import add, saved_layer, script_executable, wait_for

CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
DOCK = 'TopologyRulesDock'


def docks(window: QMainWindow) -> list[QDockWidget]:
    """The docks of the plugin that the window still has."""
    return [
        found
        for found in window.findChildren(QDockWidget)
        if found.objectName() == DOCK
    ]


@pytest.fixture
def plugin(
    qgis_iface: QgisInterface, executable: str, qgis_processing: None
) -> c.Iterator[TopologyRules]:
    plugin = TopologyRules(qgis_iface)
    plugin.initGui()
    yield plugin
    plugin.unload()


def registered() -> bool:
    registry = QgsApplication.processingRegistry()
    return registry.providerById(PROVIDER_ID) is not None


def test_the_provider_is_registered_and_removed(
    qgis_iface: QgisInterface, executable: str, qgis_processing: None
):
    plugin = TopologyRules(qgis_iface)
    plugin.initGui()
    assert registered()
    plugin.unload()
    assert not registered()


def test_reloading_twice(
    qgis_iface: QgisInterface, executable: str, qgis_processing: None
):
    for _ in range(3):
        plugin = TopologyRules(qgis_iface)
        plugin.initGui()
        assert registered()
        plugin.unload()
        assert not registered()


def test_the_dock_follows_the_setting_after_the_options_dialog(
    plugin: TopologyRules,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    executable: str,
):
    configure(tmp_path / 'missing')
    plugin.provider.refreshAlgorithms()
    plugin.dock.refresh()
    assert not plugin.dock.setup_page.isHidden()
    configure(executable)
    # What the options dialog does on OK
    plugin.provider.refreshAlgorithms()
    assert plugin.dock.setup_page.isHidden()
    assert not plugin.dock.tabs.isHidden()


def test_unloading_during_a_run(
    qgis_iface: QgisInterface,
    executable: str,
    qgis_processing: None,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
):
    configure(script_executable(tmp_path, 'import time\ntime.sleep(60)\n'))
    plugin = TopologyRules(qgis_iface)
    plugin.initGui()
    dock = plugin.dock
    dock.refresh()
    layer = saved_layer(tmp_path, CHAIN)
    QgsProject.instance().addMapLayer(layer)
    add(dock, 'line/must-not-have-dangles', layer)
    dock.run_button.click()
    assert dock.task is not None
    plugin.unload()
    # The task ends and calls back into a dock that is gone
    for _ in range(100):
        QgsApplication.processEvents()
    wait_for(lambda: not QgsApplication.taskManager().activeTasks(), 30)


def test_unloading_leaves_no_dock_in_the_main_window(
    qgis_iface: QgisInterface,
    window: QMainWindow,
    executable: str,
    qgis_processing: None,
):
    # The Plugin Reloader looks right after the unload, before the event loop
    # deletes what was scheduled to be deleted
    plugin = TopologyRules(qgis_iface)
    plugin.initGui()
    assert len(docks(window)) == 1
    plugin.unload()
    assert docks(window) == []


def test_reloading_leaves_one_dock_each_time(
    qgis_iface: QgisInterface,
    window: QMainWindow,
    executable: str,
    qgis_processing: None,
):
    for _ in range(3):
        plugin = TopologyRules(qgis_iface)
        plugin.initGui()
        assert len(docks(window)) == 1
        plugin.unload()
        assert docks(window) == []


def test_loading_again_after_an_unload_has_one_dock(
    qgis_iface: QgisInterface,
    window: QMainWindow,
    executable: str,
    qgis_processing: None,
):
    first = TopologyRules(qgis_iface)
    first.initGui()
    first.unload()
    second = TopologyRules(qgis_iface)
    second.initGui()
    assert len(docks(window)) == 1
    second.unload()


# The Processing part alone, as qgis_process and scripts load it: no window


def test_the_plugin_can_start_only_its_processing_part():
    assert hasattr(TopologyRules, 'initProcessing')


def test_starting_the_processing_part_registers_the_provider(
    executable: str, qgis_processing: None
):
    plugin = TopologyRules(None)
    plugin.initProcessing()
    try:
        assert registered()
    finally:
        plugin.unload()


def test_a_processing_only_plugin_unloads_cleanly(
    executable: str, qgis_processing: None
):
    plugin = TopologyRules(None)
    plugin.initProcessing()
    plugin.unload()
    assert not registered()
