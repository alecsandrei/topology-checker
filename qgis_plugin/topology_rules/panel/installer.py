"""The dialog that installs the Python package: what is installed, from
where and into which folder, then the install as it goes."""

from __future__ import annotations

import html
from pathlib import Path

from qgis.core import QgsApplication
from qgis.gui import QgsCollapsibleGroupBoxBasic
from qgis.PyQt.QtCore import Qt, QUrl, pyqtSignal, pyqtSlot
from qgis.PyQt.QtGui import QDesktopServices, QFontDatabase
from qgis.PyQt.QtWidgets import (
    QApplication,
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPlainTextEdit,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..algorithms.base import DOCS
from ..core import engine, install
from ..core.engine import Engine
from ..core.wheels import PACKAGE
from ..exceptions import PackageError
from ..utils import not_none, tr
from .menu import choose

# The part of the documentation with the install by hand
GUIDE = f'{DOCS}qgis-plugin/#install'
# The lines of the command box
COMMAND_LINES = 4
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


def fixed_width(field: QLineEdit | QPlainTextEdit) -> None:
    field.setFont(QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont))


def fact(text: str = '') -> QLineEdit:
    """A fact that can be long, such as a URL: it scrolls, and can be
    copied but not changed."""
    field = QLineEdit(text)
    field.setReadOnly(True)
    return field


class InstallDialog(QDialog):
    """Finds the wheel on PyPI and says what it is, then installs it when
    asked, with the lines of the install as it goes. It ends with what to do
    next, and on a failure with the install by hand."""

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

        self.status = QLabel()
        self.status.setWordWrap(True)
        self.status.setTextFormat(Qt.TextFormat.RichText)
        self.status.setOpenExternalLinks(True)
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
        self.command = QPlainTextEdit(install.command(self.version))
        self.command.setReadOnly(True)
        fixed_width(self.command)
        self.command.setFixedHeight(
            COMMAND_LINES * self.command.fontMetrics().lineSpacing()
        )
        self.copy_command = QPushButton(tr('Copy'))
        inside = QHBoxLayout(self.command_box)
        inside.addWidget(self.command, 1)
        inside.addWidget(self.copy_command, 0, Qt.AlignmentFlag.AlignTop)
        self.command_box.setCollapsed(True)

        self.log = QPlainTextEdit()
        self.log.setReadOnly(True)
        self.log.hide()

        self.buttons = QDialogButtonBox()
        self.copy_log = not_none(
            self.buttons.addButton(
                tr('Copy log'), QDialogButtonBox.ButtonRole.ActionRole
            )
        )
        self.copy_log.hide()
        self.install_button = not_none(
            self.buttons.addButton(
                tr('Update') if update else tr('Install'),
                QDialogButtonBox.ButtonRole.AcceptRole,
            )
        )
        self.cancel_button = not_none(
            self.buttons.addButton(QDialogButtonBox.StandardButton.Cancel)
        )

        layout = QVBoxLayout(self)
        layout.addWidget(self.status)
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
        task = install.InstallTask(self.release)
        task.done.connect(self.install_done)
        self.run(task)

    @pyqtSlot(str)
    def install_done(self, version: str) -> None:
        self.task = None
        self.installing = False
        # Installing is asking for the package
        if engine.chosen() is Engine.EXECUTABLE:
            choose(Engine.AUTOMATIC)
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
        not."""
        self.task = None
        self.installing = False
        self.say(error, link=True)
        self.facts.hide()
        self.install_button.hide()
        self.command_box.show()
        self.command_box.setCollapsed(False)
        self.finish()

    def finish(self) -> None:
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
            self.stop()
            self.cancel_button.setEnabled(False)
            self.say(tr('Cancelling…'))
            return
        self.stop()
        super().reject()

    def open_folder(self) -> None:
        folder = existing(install.folder_of(self.version))
        QDesktopServices.openUrl(QUrl.fromLocalFile(str(folder)))
