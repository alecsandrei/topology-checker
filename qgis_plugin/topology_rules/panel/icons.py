from __future__ import annotations

from qgis.core import QgsApplication
from qgis.PyQt.QtGui import QIcon
from qgis.PyQt.QtWidgets import QApplication, QPushButton, QStyle

from ..utils import not_none

# The icon of QGIS for each button, and the one of the style when the theme
# has none by that name
ICONS = {
    'add': ('mActionAdd', QStyle.StandardPixmap.SP_FileDialogNewFolder),
    'remove': ('symbologyRemove', QStyle.StandardPixmap.SP_DialogCloseButton),
    'clear': ('mActionDeleteSelected', QStyle.StandardPixmap.SP_TrashIcon),
    'import': (
        'mActionSharingImport',
        QStyle.StandardPixmap.SP_DialogOpenButton,
    ),
    'export': (
        'mActionSharingExport',
        QStyle.StandardPixmap.SP_DialogSaveButton,
    ),
    'run': ('mActionStart', QStyle.StandardPixmap.SP_MediaPlay),
    'stop': ('mActionStop', QStyle.StandardPixmap.SP_MediaStop),
    'help': ('mActionHelpContents', QStyle.StandardPixmap.SP_DialogHelpButton),
    'menu': ('mActionOptions', QStyle.StandardPixmap.SP_TitleBarMenuButton),
    'close': ('mIconClose', QStyle.StandardPixmap.SP_TitleBarCloseButton),
    # The rule that a run checks now
    'running': ('mTaskRunning', QStyle.StandardPixmap.SP_BrowserReload),
}


def icon(name: str) -> QIcon:
    theme, fallback = ICONS[name]
    found = QgsApplication.getThemeIcon(f'/{theme}.svg')
    if found.isNull():
        return not_none(QApplication.style()).standardIcon(fallback)
    return found


def decorate(button: QPushButton, name: str, tip: str) -> None:
    """Gives the button an icon and a tip."""
    button.setIcon(icon(name))
    button.setToolTip(tip)
