from __future__ import annotations

import string
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

    def say_about(self, template: str, name: str, **values: object) -> None:
        """Says the template with the values in its fields, and the name
        where it has the field rule."""
        self.say(*filled(template, name, values))

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


def filled(
    template: str, name: str, values: t.Mapping[str, object]
) -> tuple[str, str, str]:
    """The text of the template before the field rule, the name, and the
    text after it, with the values in the other fields. A template without
    the field, such as a translation that left it out, or that cannot be
    read, is followed by the name."""
    parts: list[list[str]] = [[]]
    try:
        for text, field, spec, conversion in string.Formatter().parse(template):
            parts[-1].append(text)
            if field is None:
                continue
            if field == 'rule':
                if len(parts) == 1:
                    parts.append([])
                else:
                    parts[-1].append(name)
            elif field in values:
                value = values[field]
                if conversion == 'r':
                    value = repr(value)
                elif conversion == 'a':
                    value = ascii(value)
                elif conversion == 's':
                    value = str(value)
                parts[-1].append(format(value, spec or ''))
            else:
                parts[-1].append(f'{{{field}}}')
    except (ValueError, TypeError):
        parts = [[template]]
    if len(parts) == 1:
        return ''.join(parts[0]) + ' ', name, ''
    before, after = parts
    return ''.join(before), name, ''.join(after)
