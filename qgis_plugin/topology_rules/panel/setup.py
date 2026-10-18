from __future__ import annotations

import html

from processing.core.ProcessingConfig import ProcessingConfig
from qgis.core import QgsSettings
from qgis.PyQt.QtWidgets import (
    QFileDialog,
    QLabel,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..enums import Setting
from ..provider import SETTING_NAME
from ..utils import tr

INSTALL = 'https://alecsandrei.github.io/topology-checker/stable/install/'


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
        # The setting of the Processing options, which they write back
        # from what they hold when the dialog is accepted
        if SETTING_NAME in ProcessingConfig.settings:
            ProcessingConfig.setSettingValue(SETTING_NAME, path)
        QgsSettings().setValue(Setting.EXECUTABLE.value, path)
        return True
