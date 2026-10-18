from __future__ import annotations

from processing.core.ProcessingConfig import ProcessingConfig, Setting
from qgis.core import (
    Qgis,
    QgsApplication,
    QgsMessageLog,
    QgsProcessingProvider,
    QgsTask,
)
from qgis.PyQt import sip
from qgis.PyQt.QtGui import QIcon

from . import probing
from .algorithms import (
    CheckDefinition,
    CheckRulesAlgorithm,
    ExplodeLines,
    MergeLines,
    RuleAlgorithm,
)
from .core import binary
from .core.binary import Rule
from .enums import PROVIDER_ID, Asset
from .enums import Setting as PluginSetting
from .exceptions import ExecutableError

SETTING_NAME = PluginSetting.EXECUTABLE.value.rsplit('/', 1)[-1]


class Provider(QgsProcessingProvider):
    # The executable that is being asked for its rules
    probing: tuple[str, int] | None = None
    # Python drops the callback of a task whose wrapper it collects
    task: QgsTask | None = None

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
            path, modified = binary.locate()
        except ExecutableError as error:
            self.log(str(error))
            return
        rules = binary.probe_cache.peek(path, modified)
        if rules is None:
            self.ask(path, modified)
            return
        for rule in rules:
            self.addAlgorithm(RuleAlgorithm(rule))
        self.addAlgorithm(CheckRulesAlgorithm())
        self.addAlgorithm(CheckDefinition())
        self.addAlgorithm(MergeLines())
        self.addAlgorithm(ExplodeLines())

    def log(self, text: str) -> None:
        QgsMessageLog.logMessage(text, self.name(), Qgis.MessageLevel.Warning)

    def ask(self, path: str, modified: int) -> None:
        """Asks the executable for its rules. The first answer can take
        seconds, which the window must not wait for, so it is asked in a
        task and the algorithms are loaded when it answers. A program
        that runs and ends (qgis_process) has no event loop to wait with, and
        asks at once."""
        if QgsApplication.platform() == 'qgis_process':
            try:
                binary.probe(path, modified)
            except ExecutableError as error:
                self.log(str(error))
                return
            self.loadAlgorithms()
            return
        if self.probing == (path, modified):
            return
        self.probing = (path, modified)
        self.task = probing.ask_in_task(
            path,
            modified,
            lambda rules, error: self.answered((path, modified), rules, error),
        )

    def answered(
        self,
        asked: tuple[str, int],
        rules: tuple[Rule, ...] | None,
        error: Exception | None,
    ) -> None:
        """The answer of the executable, back on the main thread."""
        if sip.isdeleted(self) or self.probing != asked:
            return
        self.probing = self.task = None
        if rules is None:
            self.log(str(error or 'topology-checker gave no answer'))
            return
        self.refreshAlgorithms()
