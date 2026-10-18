from __future__ import annotations

import html

from qgis.core import QgsSettings
from qgis.PyQt.QtCore import QCoreApplication
from qgis.PyQt.QtWidgets import (
    QFileDialog,
    QLabel,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..enums import Setting

INSTALL = 'https://alecsandrei.github.io/topology-checker/stable/install/'


def tr(text: str) -> str:
    return QCoreApplication.translate('TopologyRules', text)


class SetupPage(QWidget):
    """Says why the plugin cannot run, with the way to fix it: install the
    tool, or point the plugin to it."""

    def __init__(self, parent: QWidget | None = None):
        super().__init__(parent)
        self.status = QLabel()
        self.status.setWordWrap(True)
        self.status.setOpenExternalLinks(True)
        self.browse_button = QPushButton(tr('Choose topology-checker…'))
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.status)
        layout.addWidget(self.browse_button)
        layout.addStretch(1)

    def checking(self) -> None:
        self.status.setText(tr('Checking topology-checker…'))
        self.browse_button.hide()

    def explain(self, reason: str) -> None:
        self.browse_button.show()
        self.status.setText(
            f'{html.escape(reason)}<br><a href="{INSTALL}">'
            f'{tr("How to install topology-checker")}</a>'
        )

    def choose(self) -> bool:
        """Asks for the executable and remembers it, true if one was chosen."""
        path, _ = QFileDialog.getOpenFileName(
            self, tr('Choose topology-checker')
        )
        if not path:
            return False
        QgsSettings().setValue(Setting.EXECUTABLE.value, path)
        return True
