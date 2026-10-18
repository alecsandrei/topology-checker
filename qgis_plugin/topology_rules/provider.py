from __future__ import annotations

from processing.core.ProcessingConfig import ProcessingConfig, Setting
from qgis.core import Qgis, QgsMessageLog, QgsProcessingProvider
from qgis.PyQt.QtGui import QIcon

from .algorithms import (
    CheckDefinition,
    CheckRulesAlgorithm,
    ExplodeLines,
    MergeLines,
    RuleAlgorithm,
)
from .core import binary
from .enums import PROVIDER_ID, Asset
from .enums import Setting as PluginSetting
from .exceptions import ExecutableError

SETTING_NAME = PluginSetting.EXECUTABLE.value.rsplit('/', 1)[-1]


class Provider(QgsProcessingProvider):
    def id(self) -> str:
        return PROVIDER_ID

    def name(self) -> str:
        return 'Topology Rules'

    def icon(self) -> QIcon:
        return QIcon(Asset.ICON.value.as_posix())

    def load(self) -> bool:
        ProcessingConfig.settingIcons[self.name()] = self.icon()
        ProcessingConfig.addSetting(
            Setting(
                self.name(),
                SETTING_NAME,
                'Path to the topology-checker executable',
                '',
                valuetype=Setting.FILE,
            )
        )
        ProcessingConfig.readSettings()
        self.refreshAlgorithms()
        return True

    def unload(self) -> None:
        ProcessingConfig.removeSetting(SETTING_NAME)

    def loadAlgorithms(self) -> None:
        # Without the executable there are no rules to list, so the toolbox
        # stays empty until its path is set
        try:
            rules = binary.rules()
        except ExecutableError as error:
            QgsMessageLog.logMessage(
                str(error), self.name(), Qgis.MessageLevel.Warning
            )
            return
        for rule in rules:
            self.addAlgorithm(RuleAlgorithm(rule))
        self.addAlgorithm(CheckRulesAlgorithm())
        self.addAlgorithm(CheckDefinition())
        self.addAlgorithm(MergeLines())
        self.addAlgorithm(ExplodeLines())
