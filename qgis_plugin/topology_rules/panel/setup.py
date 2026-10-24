from __future__ import annotations

from processing.core.ProcessingConfig import ProcessingConfig
from qgis.core import QgsSettings
from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtWidgets import (
    QFileDialog,
    QLabel,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..core import binary
from ..enums import SETTING_NAME, Setting
from ..exceptions import ExecutableError
from ..utils import tr

INSTALL = 'https://alecsandrei.github.io/topology-checker/stable/install/'


class SetupPage(QWidget):
    """Says why the plugin cannot run, with the ways to fix it: install the
    Python package, point the plugin to the executable, or read the guide."""

    def __init__(self, parent: QWidget | None = None):
        super().__init__(parent)
        self.status = QLabel()
        self.status.setWordWrap(True)
        # A reason can hold a path, or quotes, as they are
        self.status.setTextFormat(Qt.TextFormat.PlainText)
        self.install_button = QPushButton(tr('Install Python package…'))
        self.browse_button = QPushButton(tr('Choose executable…'))
        self.guide = QLabel(
            f'<a href="{INSTALL}">{tr("How to install topology-checker")}</a>'
        )
        self.guide.setOpenExternalLinks(True)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.status)
        layout.addWidget(self.install_button)
        layout.addWidget(self.browse_button)
        layout.addWidget(self.guide)
        layout.addStretch(1)

    def checking(self) -> None:
        self.status.setText(tr('Checking topology-checker…'))
        for widget in (self.install_button, self.browse_button, self.guide):
            widget.hide()

    def explain(self, reason: str, installable: bool = False) -> None:
        """Says why, and offers the install where the package can be
        installed."""
        self.status.setText(reason)
        self.install_button.setVisible(installable)
        self.install_button.setDefault(installable)
        self.browse_button.show()
        self.guide.show()

    def choose(self) -> bool:
        """Asks for the executable and remembers it, true if one was chosen.
        The dialog opens in the folder of the executable in use."""
        try:
            folder = str(binary.find_executable().parent)
        except ExecutableError:
            folder = ''
        path, _ = QFileDialog.getOpenFileName(
            self, tr('Choose topology-checker'), folder
        )
        if not path:
            return False
        # The setting of the Processing options, which they write back
        # from what they hold when the dialog is accepted
        if SETTING_NAME in ProcessingConfig.settings:
            ProcessingConfig.setSettingValue(SETTING_NAME, path)
        QgsSettings().setValue(Setting.EXECUTABLE.value, path)
        return True
