"""The dialog that installs the Python package: what is installed, from
where and into which folder, then the install as it goes."""

from __future__ import annotations

import html
import math
from pathlib import Path

from qgis.core import QgsApplication
from qgis.gui import QgsCollapsibleGroupBoxBasic
from qgis.PyQt.QtCore import Qt, QUrl, pyqtSignal, pyqtSlot
from qgis.PyQt.QtGui import QDesktopServices, QFontDatabase, QTextOption
from qgis.PyQt.QtWidgets import (
    QApplication,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QProgressBar,
    QPushButton,
    QSizePolicy,
    QTextEdit,
    QVBoxLayout,
    QWidget,
)

from ..algorithms.base import DOCS
from ..core import engine, install
from ..core.engine import Engine
from ..core.wheels import PACKAGE
from ..exceptions import PackageError
from ..utils import not_none, tr
from .menu import relist

# The part of the documentation with the install by hand
GUIDE = f'{DOCS}qgis-plugin/#install'
# The lines that the log shows without scrolling: all of an install
LOG_LINES = 10
# The tasks that run, kept until they end: a dialog that closes does not
# wait for its task, and Python must not collect it before QGIS is done
running: set[install.Task] = set()


def copy(text: str) -> None:
    not_none(QApplication.clipboard()).setText(text)


def existing(path: Path) -> Path:
    """The folder, or the nearest one above it that exists."""
    while not path.exists() and path.parent != path:
        path = path.parent
    return path


def fixed_width(field: QLineEdit | QPlainTextEdit | QTextEdit) -> None:
    field.setFont(QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont))


def fact(text: str = '') -> QLineEdit:
    """A fact that can be long, such as a URL: it scrolls, and can be
    copied but not changed."""
    field = QLineEdit(text)
    field.setReadOnly(True)
    return field


class Log(QPlainTextEdit):
    """The lines of the install, which stay at the end as they come and as
    the dialog changes around them."""

    def __init__(self) -> None:
        super().__init__()
        self.setReadOnly(True)
        margin = not_none(self.document()).documentMargin()
        self.setMinimumHeight(
            math.ceil(
                LOG_LINES * self.fontMetrics().lineSpacing()
                + 2 * (margin + self.frameWidth())
            )
        )
        bar = not_none(self.verticalScrollBar())
        bar.rangeChanged.connect(lambda _, end: bar.setValue(end))

    def to_end(self) -> None:
        bar = not_none(self.verticalScrollBar())
        bar.setValue(bar.maximum())


class Command(QTextEdit):
    """The command, as high as all of its lines. They wrap anywhere, as a
    path has no spaces to wrap at."""

    def __init__(self, text: str) -> None:
        super().__init__()
        self.setAcceptRichText(False)
        self.setPlainText(text)
        self.setReadOnly(True)
        self.setWordWrapMode(QTextOption.WrapMode.WrapAtWordBoundaryOrAnywhere)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.setSizePolicy(
            QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed
        )
        document = not_none(self.document())
        layout = not_none(document.documentLayout())
        layout.documentSizeChanged.connect(self.fit)
        self.fit()

    def fit(self) -> None:
        height = not_none(self.document()).size().height()
        self.setFixedHeight(math.ceil(height) + 2 * self.frameWidth())


class InstallDialog(QDialog):
    """Finds the wheel on PyPI and says what it is, then installs it when
    asked, with the lines of the install as it goes. It ends with what to do
    next, and on a failure with the install by hand and a retry."""

    # The version, once the toolbox lists the rules of the engine
    installed = pyqtSignal(str)

    def __init__(self, parent: QWidget | None = None, update: bool = False):
        super().__init__(parent)
        self.setWindowTitle(
            tr('Update Python package')
            if update
            else tr('Install Python package')
        )
        self.version = install.wanted()
        self.release: install.Release | None = None
        self.task: install.Task | None = None
        self.installing = False
        self.cancelled = False

        self.status = QLabel()
        self.status.setWordWrap(True)
        self.status.setTextFormat(Qt.TextFormat.RichText)
        self.status.setOpenExternalLinks(True)
        self.progress = QProgressBar()
        self.progress.setRange(0, 100)
        self.progress.hide()
        self.package_label = QLabel(f'{PACKAGE} {self.version}')
        self.file_field = fact()
        self.source_field = fact()
        self.sha256_field = fact()
        fixed_width(self.sha256_field)
        self.folder_field = fact(str(install.folder_of(self.version)))
        self.open_button = QPushButton(tr('Open'))
        # The package needs no other package
        self.dependencies_label = QLabel(tr('None'))
        self.facts = QWidget()
        form = QFormLayout(self.facts)
        form.setContentsMargins(0, 0, 0, 0)
        form.addRow(tr('Package'), self.package_label)
        form.addRow(tr('File'), self.file_field)
        form.addRow(tr('Source'), self.source_field)
        form.addRow(tr('SHA-256'), self.sha256_field)
        folder_row = QHBoxLayout()
        folder_row.addWidget(self.folder_field, 1)
        folder_row.addWidget(self.open_button)
        form.addRow(tr('Folder'), folder_row)
        form.addRow(tr('Dependencies'), self.dependencies_label)

        self.command_box = QgsCollapsibleGroupBoxBasic(
            tr('Command for {terminal}').format(terminal=install.terminal())
        )
        self.command = Command(install.command(self.version))
        fixed_width(self.command)
        self.copy_command = QPushButton(tr('Copy'))
        inside = QHBoxLayout(self.command_box)
        inside.addWidget(self.command, 1)
        inside.addWidget(self.copy_command, 0, Qt.AlignmentFlag.AlignTop)
        self.command_box.setCollapsed(True)

        self.log = Log()
        self.log.hide()

        self.buttons = QDialogButtonBox()
        self.copy_log = not_none(
            self.buttons.addButton(
                tr('Copy log'), QDialogButtonBox.ButtonRole.ActionRole
            )
        )
        self.retry_button = not_none(
            self.buttons.addButton(
                tr('Retry'), QDialogButtonBox.ButtonRole.ActionRole
            )
        )
        self.install_button = not_none(
            self.buttons.addButton(
                tr('Update') if update else tr('Install'),
                QDialogButtonBox.ButtonRole.AcceptRole,
            )
        )
        self.cancel_button = not_none(
            self.buttons.addButton(QDialogButtonBox.StandardButton.Cancel)
        )
        # Once all are in: Qt 5 shows every button again as one is added
        self.copy_log.hide()
        self.retry_button.hide()

        layout = QVBoxLayout(self)
        layout.addWidget(self.status)
        layout.addWidget(self.progress)
        layout.addWidget(self.facts)
        layout.addWidget(self.log, 1)
        layout.addWidget(self.command_box)
        layout.addWidget(self.buttons)
        self.setMinimumWidth(560)

        self.open_button.clicked.connect(self.open_folder)
        self.copy_command.clicked.connect(
            lambda: copy(self.command.toPlainText())
        )
        self.copy_log.clicked.connect(lambda: copy(self.log.toPlainText()))
        self.retry_button.clicked.connect(self.retry)
        self.buttons.accepted.connect(self.start)
        self.buttons.rejected.connect(self.reject)
        self.look_up()

    def say(self, text: str, link: bool = False) -> None:
        """Shows a line over the rest, with the install by hand after it."""
        guide = f'<br><a href="{GUIDE}">{tr("Install by hand")}</a>'
        self.status.setText(html.escape(text) + (guide if link else ''))
        self.status.show()

    # The states

    def look_up(self) -> None:
        """Asks PyPI for the wheel of this QGIS, to show what would be
        installed."""
        self.say(
            tr('Looking up {package} {version}…').format(
                package=PACKAGE, version=self.version
            )
        )
        self.facts.hide()
        self.install_button.show()
        self.install_button.setEnabled(False)
        task = install.ReleaseTask(self.version)
        task.found.connect(self.found)
        self.run(task)

    @pyqtSlot(object)
    def found(self, release: install.Release) -> None:
        self.task = None
        self.release = release
        self.status.hide()
        self.file_field.setText(
            f'{release.filename} ({install.size_text(release.size)})'
        )
        self.source_field.setText(release.url)
        self.sha256_field.setText(release.sha256)
        for field in (self.file_field, self.source_field, self.sha256_field):
            field.setCursorPosition(0)
        self.facts.show()
        self.install_button.setEnabled(True)
        self.install_button.setDefault(True)
        self.install_button.setFocus()

    def start(self) -> None:
        """Installs the wheel that was found, with its lines shown."""
        if self.release is None or self.task is not None:
            return
        self.installing = True
        self.cancelled = False
        self.say(
            tr('Installing {package} {version}…').format(
                package=PACKAGE, version=self.release.version
            )
        )
        self.facts.hide()
        self.command_box.hide()
        self.install_button.hide()
        self.log.show()
        self.copy_log.show()
        self.progress.setValue(0)
        self.progress.show()
        task = install.InstallTask(self.release)
        task.done.connect(self.install_done)
        task.progressChanged.connect(self.progressed)
        self.run(task)

    @pyqtSlot(float)
    def progressed(self, value: float) -> None:
        self.progress.setValue(round(value))

    @pyqtSlot(str)
    def install_done(self, version: str) -> None:
        self.task = None
        self.installing = False
        # Installing is asking for the package
        relist(
            Engine.AUTOMATIC if engine.chosen() is Engine.EXECUTABLE else None
        )
        self.installed.emit(version)
        self.say(self.outcome(version))
        self.finish()

    def outcome(self, version: str) -> str:
        """What the install did, and what is left to do."""
        name = f'{PACKAGE} {version}'
        if install.restart_needed():
            return tr('Installed {name}. Restart QGIS to use it.').format(
                name=name
            )
        try:
            engine.find_package()
        except PackageError as error:
            return tr('Installed {name}. {error}').format(
                name=name, error=error
            )
        return tr('Installed {name}. In use now.').format(name=name)

    @pyqtSlot(str)
    def failed(self, error: str) -> None:
        """Says why, with the install by hand, which may work where this did
        not. What was cancelled only says so."""
        # A QGIS that cannot use the package fails again
        retry = not (
            isinstance(self.task, install.ReleaseTask)
            and self.task.reason is not None
        )
        self.task = None
        self.installing = False
        if self.cancelled:
            self.say(tr('Cancelled.'))
        else:
            self.say(error, link=True)
        self.facts.hide()
        self.install_button.hide()
        self.log.show()
        self.copy_log.show()
        self.retry_button.setVisible(retry)
        self.command_box.show()
        self.command_box.setCollapsed(self.cancelled)
        self.finish()

    def retry(self) -> None:
        """Starts again from what failed: the look-up or the install."""
        self.retry_button.hide()
        self.log.clear()
        self.command_box.setCollapsed(True)
        self.cancel_button.setText(tr('Cancel'))
        if self.release is None:
            self.log.hide()
            self.copy_log.hide()
            self.look_up()
        else:
            self.start()

    def finish(self) -> None:
        self.progress.hide()
        self.log.to_end()
        self.cancel_button.setText(tr('Close'))
        self.cancel_button.setEnabled(True)
        self.cancel_button.setDefault(True)
        self.cancel_button.setFocus()

    # The tasks

    def run(self, task: install.Task) -> None:
        self.task = task
        running.add(task)
        task.logged.connect(self.logged)
        task.failed.connect(self.failed)
        for signal in (task.taskCompleted, task.taskTerminated):
            signal.connect(lambda: running.discard(task))
        not_none(QgsApplication.taskManager()).addTask(task)

    @pyqtSlot(str)
    def logged(self, line: str) -> None:
        self.log.appendPlainText(line)

    def stop(self) -> None:
        """Cancels what runs."""
        if self.task is not None:
            self.task.cancel()

    def reject(self) -> None:
        """Closes, but an install that runs is cancelled first, and stays to
        say when it stopped."""
        if self.installing:
            self.cancelled = True
            self.stop()
            self.cancel_button.setEnabled(False)
            self.say(tr('Cancelling…'))
            return
        self.stop()
        super().reject()

    def open_folder(self) -> None:
        folder = existing(install.folder_of(self.version))
        QDesktopServices.openUrl(QUrl.fromLocalFile(str(folder)))
