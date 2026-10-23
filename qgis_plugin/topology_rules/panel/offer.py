"""The bar that offers the Python package once, where the executable runs
the checks only because the package is not installed."""

from __future__ import annotations

from qgis.core import QgsSettings
from qgis.PyQt.QtCore import pyqtSignal
from qgis.PyQt.QtWidgets import (
    QFrame,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QToolButton,
    QWidget,
)

from ..core import engine
from ..core.engine import Engine
from ..utils import tr
from .icons import icon
from .menu import installable

# Whether the package was offered and then installed or turned down, in the
# settings of the user
OFFERED = 'topology_rules/package_offered'


def wanted() -> bool:
    """Whether to offer the package: it was not turned down, Automatic would
    take it, and this QGIS can install it. The cheap checks come first."""
    return (
        not QgsSettings().value(OFFERED, False, type=bool)
        and engine.chosen() is Engine.AUTOMATIC
        and installable()
    )


def stop_offering() -> None:
    QgsSettings().setValue(OFFERED, True)


class OfferBar(QFrame):
    """'A faster engine is available', with Install and a button that closes
    it for good."""

    install_clicked = pyqtSignal()

    def __init__(self, parent: QWidget | None = None):
        super().__init__(parent)
        self.setFrameShape(QFrame.Shape.StyledPanel)
        self.label = QLabel(tr('A faster engine is available'))
        self.label.setWordWrap(True)
        self.install_button = QPushButton(tr('Install…'))
        self.install_button.setToolTip(tr('Install the Python package'))
        self.close_button = QToolButton()
        self.close_button.setIcon(icon('close'))
        self.close_button.setAutoRaise(True)
        self.close_button.setToolTip(tr('Do not show again'))
        row = QHBoxLayout(self)
        row.setContentsMargins(6, 2, 2, 2)
        row.addWidget(self.label, 1)
        row.addWidget(self.install_button)
        row.addWidget(self.close_button)
        self.hide()

        self.install_button.clicked.connect(self.install_clicked)
        self.close_button.clicked.connect(self.dismiss)

    def update_offer(self) -> None:
        self.setVisible(wanted())

    def dismiss(self) -> None:
        stop_offering()
        self.hide()
