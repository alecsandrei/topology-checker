from __future__ import annotations

import collections.abc as c
import typing as t

from qgis.core import Qgis
from qgis.gui import QgsMessageBar, QgsMessageBarItem
from qgis.PyQt.QtCore import Qt, pyqtSignal
from qgis.PyQt.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QLabel,
    QPlainTextEdit,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..core.progress import Checked, Progress, Stage
from ..utils import not_none, tr
from .ignored import stamp
from .rows import Row, found_rows

# How long a message that is not a failure stays
SECONDS = 5
# Marks the failure that the plugin shows in the message bar
MARK = 'topology_rules/failure'


Staged = tuple[Stage, int | None, int]
# A rule that is done, with its errors as the tree shows them
Done = tuple[Checked, list[Row]]


class RunFeedback(Progress):
    """Feedback of a run that remembers the errors that were reported, and
    tells the panel the stage of the run and each rule that is done. The
    signals come from the thread of the run."""

    stage_changed = pyqtSignal(object)
    rule_done = pyqtSignal(object)

    def __init__(self, names: c.Mapping[str, str] | None = None) -> None:
        super().__init__()
        self.errors: list[str] = []
        # The names of the layers of the run, by their ids
        self.names = dict(names or {})

    def staged(self, stage: Stage, index: int | None, total: int) -> None:
        staged: Staged = (stage, index, total)
        self.stage_changed.emit(staged)

    def checked(self, done: Checked) -> None:
        # The rows are made here, so that the panel only has to show them
        found: Done = (done, stamp(found_rows(done, self.names), done.layers))
        self.rule_done.emit(found)

    def reportError(self, error: str | None, fatalError: bool = False) -> None:
        self.errors.append(str(error))
        super().reportError(error, fatalError)


def show_log(parent: QWidget, log: str) -> None:
    dialog = QDialog(parent)
    dialog.setWindowTitle(tr('Topology Rules'))
    text = QPlainTextEdit(log)
    text.setReadOnly(True)
    buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Close)
    buttons.rejected.connect(dialog.reject)
    layout = QVBoxLayout(dialog)
    layout.addWidget(text)
    layout.addWidget(buttons)
    dialog.resize(700, 400)
    dialog.exec()


def forget(bar: QgsMessageBar) -> None:
    """Takes the messages of the plugin off the bar, so that the newest is
    the only one."""
    for old in bar.findChildren(QgsMessageBarItem):
        if old.property(MARK):
            bar.popWidget(old)
            # Gone from the bar now, not when the event loop deletes it
            t.cast(t.Any, old).setParent(None)
            old.deleteLater()


def push_failure(bar: QgsMessageBar, error: str, log: str) -> None:
    """Says what went wrong, with the full log behind a button."""
    forget(bar)
    item = not_none(bar.createMessage(tr('Topology Rules'), ''))
    item.setProperty(MARK, True)
    label = QLabel(error)
    label.setTextFormat(Qt.TextFormat.PlainText)
    label.setWordWrap(True)
    button = QPushButton(tr('Details'))
    button.clicked.connect(lambda: show_log(bar, log))
    layout = not_none(item.layout())
    # The layout owns the widgets from here on
    layout.addWidget(label)
    layout.addWidget(button)
    bar.pushWidget(item, Qgis.MessageLevel.Critical)


def push_summary(
    bar: QgsMessageBar,
    text: str,
    level: t.Any,
    action: tuple[str, t.Callable[[], None]] | None = None,
) -> None:
    """Says how a run went, for a few seconds, with a button for the action
    if there is one."""
    forget(bar)
    bar.pushMessage(tr('Topology Rules'), text, level, SECONDS)
    if action is None:
        return
    button = QPushButton(action[0])
    button.clicked.connect(action[1])
    item = bar.currentItem()
    if item is None or item.text() != text:
        # The bar did not show the message, so the button gets one
        item = not_none(bar.createMessage('', ''))
        bar.pushWidget(item, level, SECONDS)
    item.setProperty(MARK, True)
    not_none(item.layout()).addWidget(button)
