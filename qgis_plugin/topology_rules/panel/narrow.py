from __future__ import annotations

from qgis.PyQt.QtCore import QSize, Qt
from qgis.PyQt.QtWidgets import (
    QComboBox,
    QPushButton,
    QSizePolicy,
    QTabWidget,
)

# How many characters a box of choices keeps room for, however long its items
CHARACTERS = 12


def shrink_buttons(*buttons: QPushButton) -> None:
    """Lets buttons in a row be narrower than their text and icon, so that a
    narrow dock is not made wider by them: they share the room."""
    for button in buttons:
        button.setSizePolicy(
            QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Fixed
        )


def shrink_combo(box: QComboBox) -> None:
    """Keeps a box of choices as narrow as a few characters, and not as wide
    as its longest item, which differs between systems and fonts."""
    box.setSizeAdjustPolicy(
        QComboBox.SizeAdjustPolicy.AdjustToMinimumContentsLengthWithIcon
    )
    box.setMinimumContentsLength(CHARACTERS)
    box.setSizePolicy(QSizePolicy.Policy.Ignored, QSizePolicy.Policy.Fixed)


def shrink_tabs(tabs: QTabWidget) -> None:
    """Lets the tabs be cut short, with a button to scroll to the rest, and
    not ask for the width of all their text."""
    bar = tabs.tabBar()
    if bar is not None:
        bar.setElideMode(Qt.TextElideMode.ElideRight)
        bar.setUsesScrollButtons(True)


def narrowest(size: QSize, width: int) -> QSize:
    """The size with a width of at most the one given."""
    return QSize(min(size.width(), width), size.height())
