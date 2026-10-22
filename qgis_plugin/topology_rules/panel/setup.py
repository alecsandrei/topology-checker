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

from ..enums import SETTING_NAME, Setting
from ..utils import tr

INSTALL = 'https://alecsandrei.github.io/topology-checker/stable/install/'
RELEASES = 'https://github.com/alecsandrei/topology-checker/releases'
PYPI = 'https://pypi.org/project/topology-checker/'


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
        self.status.setOpenExternalLinks(True)
        escaped_reason = html.escape(reason)
        pypi_link = f'<a href="{PYPI}">{tr("PyPI")}</a>'
        releases_link = f'<a href="{RELEASES}">{tr("GitHub Releases")}</a>'
        docs_link = (
            f'<a href="{INSTALL}">{tr("How to install topology-checker")}</a>'
        )

        option_pypi = tr(
            'For maximum performance, install the Python package from {pypi} '
            'into your Python environment: '
            '<code>pip install topology-checker</code>'
        ).format(pypi=pypi_link)
        option_cli = tr(
            'Or download the standalone CLI executable from {releases} '
            'and click "Choose topology-checker…" below.'
        ).format(releases=releases_link)

        self.status.setText(
            f'<p>{escaped_reason}</p>'
            f'<p>1. {option_pypi}</p>'
            f'<p>2. {option_cli}</p>'
            f'<p>{docs_link}</p>'
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
