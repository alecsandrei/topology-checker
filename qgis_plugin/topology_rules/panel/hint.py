from __future__ import annotations

import typing as t

from qgis.PyQt.QtCore import QEvent, QObject, Qt
from qgis.PyQt.QtWidgets import QAbstractScrollArea, QLabel

from ..utils import not_none


class Hint(QLabel):
    """A grey note in the middle of an empty list, so that the list keeps its
    place and its frame, and the layout does not change when it fills."""

    def __init__(self, area: QAbstractScrollArea, text: str):
        super().__init__(text, not_none(area.viewport()))
        self.area = area
        self.setWordWrap(True)
        self.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.setMargin(8)
        self.setStyleSheet('color: palette(mid); background: transparent;')
        # The note does not take the clicks of the list
        self.setAttribute(Qt.WidgetAttribute.WA_TransparentForMouseEvents)
        self.follow = _Follow(self)
        not_none(area.viewport()).installEventFilter(self.follow)
        self.fit()

    def fit(self) -> None:
        viewport = not_none(self.area.viewport())
        self.setGeometry(0, 0, viewport.width(), viewport.height())

    def set_empty(self, empty: bool) -> None:
        self.setVisible(empty)
        if empty:
            self.raise_()


class _Follow(QObject):
    """Keeps the note the size of the list."""

    def __init__(self, hint: Hint):
        super().__init__(hint)
        self.hint = hint

    def eventFilter(self, *args: t.Any) -> bool:  # pyright: ignore[reportIncompatibleMethodOverride]
        if t.cast(QEvent, args[1]).type() == QEvent.Type.Resize:
            self.hint.fit()
        return False
