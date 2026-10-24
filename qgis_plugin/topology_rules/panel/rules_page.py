from __future__ import annotations

from qgis.core import QgsSettings
from qgis.gui import QgsCollapsibleGroupBox
from qgis.PyQt.QtCore import QSize
from qgis.PyQt.QtWidgets import (
    QCheckBox,
    QFormLayout,
    QHBoxLayout,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from ..utils import tr
from .form import RuleForm
from .icons import decorate
from .narrow import narrowest
from .offer import OfferBar
from .rule_list import RuleList
from .tolerance import tolerance_box

# Whether the box of options is closed, in the settings of the user
COLLAPSED = 'topology_rules/options_collapsed'
# The least width that the box asks for, whatever its title says
MINIMUM_WIDTH = 120


class OptionsBox(QgsCollapsibleGroupBox):
    """A box that is closed at first, and as the user left it after that."""

    def __init__(self, title: str):
        super().__init__(title)
        self.setObjectName('TopologyRulesOptions')
        self.setCollapsed(QgsSettings().value(COLLAPSED, True, type=bool))
        # A click on the title says so; setCollapsed does not
        self.collapsedStateChanged.connect(self.remember)

    def minimumSizeHint(self) -> QSize:
        """The title can say all that is set, and is long: the box is cut
        short instead of making the dock wider."""
        return narrowest(super().minimumSizeHint(), MINIMUM_WIDTH)

    def setCollapsed(self, collapse: bool) -> None:
        super().setCollapsed(collapse)
        self.remember(collapse)

    def remember(self, collapsed: bool) -> None:
        QgsSettings().setValue(COLLAPSED, collapsed)


class RulesPage(QWidget):
    """Where the rules are made and listed, and what the run is to check."""

    def __init__(self, parent: QWidget | None = None):
        super().__init__(parent)
        self.offer = OfferBar()
        self.form = RuleForm()
        self.rule_list = RuleList()
        self.import_button = QPushButton(tr('Import…'))
        self.export_button = QPushButton(tr('Export…'))
        decorate(
            self.import_button,
            'import',
            tr('Add the rules of a topology.toml to the list'),
        )
        decorate(
            self.export_button,
            'export',
            tr('Save the rules of the list to a topology.toml'),
        )
        self.selected_only = QCheckBox(tr('Selected features only'))
        self.extent_only = QCheckBox(tr('Only the visible extent'))
        self.tolerance = tolerance_box()
        self.options = OptionsBox(tr('Options'))
        inside = QVBoxLayout(self.options)
        inside.addWidget(self.selected_only)
        inside.addWidget(self.extent_only)
        tolerance = QFormLayout()
        tolerance.addRow(tr('Tolerance'), self.tolerance)
        inside.addLayout(tolerance)
        page = QVBoxLayout(self)
        # Little room is lost on the sides, as the dock is narrow
        page.setContentsMargins(0, 2, 0, 2)
        page.addWidget(self.offer)
        page.addWidget(self.form)
        page.addWidget(self.rule_list, 1)
        # The buttons share the row, and are never narrower than their text
        files_row = QHBoxLayout()
        files_row.addWidget(self.import_button, 1)
        files_row.addWidget(self.export_button, 1)
        page.addLayout(files_row)
        page.addWidget(self.options)
        for signal in (
            self.tolerance.valueChanged,
            self.selected_only.toggled,
            self.extent_only.toggled,
        ):
            signal.connect(self.update_title)
        self.update_title()

    def update_title(self) -> None:
        """Says in the title of the box what is set in it, since it is
        closed most of the time."""
        set_here: list[str] = []
        if self.tolerance.value() >= 0:
            unit = self.tolerance.suffix()
            set_here.append(
                tr('tolerance {value}').format(
                    value=f'{self.tolerance.value():g}{unit}'
                )
            )
        if self.selected_only.isChecked():
            set_here.append(tr('selected features'))
        if self.extent_only.isChecked():
            set_here.append(tr('visible extent'))
        title = ' · '.join([tr('Options'), *set_here])
        self.options.setTitle(title)
        self.options.setToolTip(title)
