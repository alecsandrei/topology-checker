from __future__ import annotations

import typing as t

from qgis.core import Qgis, QgsProcessingFeedback
from qgis.gui import QgsMessageBar, QgsMessageBarItem
from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QLabel,
    QPlainTextEdit,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..utils import not_none, tr

# How long a message that is not a failure stays
SECONDS = 5
# Marks the failure that the plugin shows in the message bar
MARK = 'topology_rules/failure'


class RunFeedback(QgsProcessingFeedback):
    """Feedback of a run that remembers the errors that were reported."""

    def __init__(self) -> None:
        super().__init__()
        self.errors: list[str] = []

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
