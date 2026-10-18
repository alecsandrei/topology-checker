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
from .utils import named, not_none

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
        self.task = named(QgsTask, 'fromFunction')(
            'Checking topology-checker',
            lambda task: probe(path, modified),
            on_finished=lambda error, result: self.answered(
                (path, modified), error, result
            ),
        )
        not_none(QgsApplication.taskManager()).addTask(self.task)

    def answered(
        self,
        asked: tuple[str, int],
        error: Exception | None,
        result: object,
    ) -> None:
        """The answer of the executable, back on the main thread."""
        if sip.isdeleted(self) or self.probing != asked:
            return
        self.probing = self.task = None
        failure = error or (result if isinstance(result, Exception) else None)
        if failure is not None or result is None:
            self.log(str(failure or 'topology-checker gave no answer'))
            return
        self.refreshAlgorithms()


def probe(path: str, modified: int) -> object:
    """The rules, or the error that is the answer: QGIS 4 does not call back
    a task whose function raised."""
    try:
        return binary.probe(path, modified)
    except Exception as error:
        return error
