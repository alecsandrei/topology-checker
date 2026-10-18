from __future__ import annotations

import typing as t

from qgis.core import QgsApplication
from qgis.gui import QgisInterface
from qgis.PyQt import sip
from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtGui import QIcon
from qgis.PyQt.QtWidgets import QAction

from .enums import PROVIDER_ID, Asset
from .panel.dock import Dock
from .provider import Provider
from .utils import not_none

MENU = '&Topology Rules'


class TopologyRules:
    def __init__(self, iface: QgisInterface | None):
        self.iface = iface
        self.provider: Provider | None = None
        self.dock: Dock | None = None
        self.action: t.Any = None

    def initProcessing(self) -> None:
        """Adds the algorithms to Processing, and nothing else: this is all
        of the plugin that a program without a window, such as
        `qgis_process`, starts."""
        registry = not_none(QgsApplication.processingRegistry())
        # A provider that a reload left behind would make QGIS refuse ours
        stale = registry.providerById(PROVIDER_ID)
        if stale is not None:
            registry.removeProvider(stale)
        self.provider = Provider()
        registry.addProvider(self.provider)

    def initGui(self) -> None:
        iface = not_none(self.iface)
        self.initProcessing()
        provider = not_none(self.provider)

        dock = Dock(iface, iface.mainWindow())
        iface.addDockWidget(Qt.DockWidgetArea.RightDockWidgetArea, dock)
        dock.hide()
        # The options dialog refreshes the provider after the path changed
        provider.algorithmsLoaded.connect(dock.refresh)

        action = QAction(
            QIcon(Asset.ICON.value.as_posix()),
            'Topology Rules',
            iface.mainWindow(),
        )
        action.setCheckable(True)
        action.toggled.connect(dock.setVisible)
        dock.visibilityChanged.connect(action.setChecked)
        iface.addToolBarIcon(action)
        iface.addPluginToVectorMenu(MENU, action)
        self.dock = dock
        self.action = action

    def unload(self) -> None:
        """Takes everything of the plugin out of QGIS, so that nothing of it
        is left under the main window when this returns."""
        if self.iface is not None and self.dock and self.action:
            self.iface.removePluginVectorMenu(MENU, self.action)
            self.iface.removeToolBarIcon(self.action)
            if self.provider is not None:
                self.provider.algorithmsLoaded.disconnect(self.dock.refresh)
            self.dock.dispose()
            self.iface.removeDockWidget(self.dock)
            # Deleted now, as deleting later would leave both under the
            # window until the event loop gets to them
            sip.delete(self.dock)
            sip.delete(self.action)
            self.dock = None
            self.action = None
        if self.provider is not None:
            not_none(QgsApplication.processingRegistry()).removeProvider(
                self.provider
            )
            self.provider = None
