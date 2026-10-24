from __future__ import annotations

import typing as t

from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtWidgets import QLabel, QSizePolicy, QWidget


class StageLabel(QLabel):
    """What a run does now, on one line however narrow the panel is: the
    name of the rule is cut short to fit, the rest of the text is not."""

    def __init__(self, parent: QWidget | None = None):
        super().__init__(parent)
        self.parts = ('', '', '')
        self.setTextFormat(Qt.TextFormat.PlainText)
        # The panel decides the width, not the text
        self.setSizePolicy(
            QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Preferred
        )
        self.setContentsMargins(4, 0, 4, 0)

    def say(self, before: str, name: str = '', after: str = '') -> None:
        self.parts = (before, name, after)
        self.setToolTip(self.full())
        self.fit()

    def say_about(self, template: str, name: str) -> None:
        """Says the template with the name where it has '{rule}'."""
        before, _, after = template.partition('{rule}')
        self.say(before, name, after)

    def full(self) -> str:
        return ''.join(self.parts)

    def fit(self) -> None:
        before, name, after = self.parts
        metrics = self.fontMetrics()
        room = self.contentsRect().width() - metrics.horizontalAdvance(
            before + after
        )
        shown = (
            metrics.elidedText(name, Qt.TextElideMode.ElideRight, max(room, 0))
            if name
            else ''
        )
        self.setText(before + shown + after)

    # Qt 5 and Qt 6 differ on whether the event can be None
    def resizeEvent(self, a0: t.Any) -> None:
        super().resizeEvent(a0)
        self.fit()
