from __future__ import annotations

from qgis.core import QgsSymbolLayerUtils
from qgis.gui import QgsColorRampButton
from qgis.PyQt.QtCore import QSize, pyqtSignal
from qgis.PyQt.QtWidgets import QActionGroup, QMenu, QWidget

from ..core.style import (
    ColourBy,
    Size,
    choose,
    choose_ramp,
    colour_by,
    colour_ramp,
    default_ramp,
    reset_style,
    size,
)
from ..enums import Setting
from ..utils import not_none, tr


class StyleMenu(QMenu):
    """How the errors are drawn, for every project: what gives them their
    colours, how big they are and the colour ramp."""

    # The user chose another style
    changed = pyqtSignal()

    def __init__(self, parent: QWidget | None = None):
        super().__init__(tr('Style'), parent)
        self.colour_menu = not_none(self.addMenu(tr('Colour by')))
        self.colours = QActionGroup(self.colour_menu)
        self.size_menu = not_none(self.addMenu(tr('Size')))
        self.sizes = QActionGroup(self.size_menu)
        colours = [
            (ColourBy.RULE_AND_LAYER, tr('Rule and layer')),
            (ColourBy.RULE, tr('Rule')),
            (ColourBy.LAYER, tr('Layer')),
            (ColourBy.ONE, tr('One colour')),
        ]
        sizes = [
            (Size.SMALL, tr('Small')),
            (Size.NORMAL, tr('Normal')),
            (Size.LARGE, tr('Large')),
        ]
        # One item of each submenu is checked
        for menu, group, options in (
            (self.colour_menu, self.colours, colours),
            (self.size_menu, self.sizes, sizes),
        ):
            for value, text in options:
                action = not_none(menu.addAction(text))
                action.setCheckable(True)
                action.setData(value.value)
                group.addAction(action)
        # The button is never shown: its menu has the colour ramps of the
        # style, the default one of the plugin, and the dialogs to edit them
        self.ramp_button = QgsColorRampButton(self, tr('Colour ramp'))
        self.ramp_button.hide()
        self.ramp_button.setDefaultColorRamp(default_ramp())
        self.ramp_menu = not_none(self.ramp_button.menu())
        self.ramp_menu.setTitle(tr('Colour ramp'))
        self.addMenu(self.ramp_menu)
        self.addSeparator()
        self.reset_action = not_none(self.addAction(tr('Reset style')))

        self.colours.triggered.connect(self.colour_chosen)
        self.sizes.triggered.connect(self.size_chosen)
        self.ramp_button.colorRampChanged.connect(self.ramp_chosen)
        self.reset_action.triggered.connect(self.reset)
        self.aboutToShow.connect(self.show_current)
        self.show_current()

    def show_current(self) -> None:
        """Checks what the user chose, which can have changed elsewhere."""
        for group, value in (
            (self.colours, colour_by()),
            (self.sizes, size()),
        ):
            for action in group.actions():
                action.setChecked(action.data() == value.value)
        ramp = colour_ramp()
        self.ramp_button.blockSignals(True)
        self.ramp_button.setColorRamp(ramp)
        self.ramp_button.blockSignals(False)
        self.ramp_menu.setIcon(
            QgsSymbolLayerUtils.colorRampPreviewIcon(ramp, QSize(16, 16))
        )

    def colour_chosen(self) -> None:
        action = not_none(self.colours.checkedAction())
        choose(Setting.COLOUR_BY, ColourBy(action.data()))
        self.changed.emit()

    def size_chosen(self) -> None:
        action = not_none(self.sizes.checkedAction())
        choose(Setting.SIZE, Size(action.data()))
        self.changed.emit()

    def ramp_chosen(self) -> None:
        ramp = self.ramp_button.colorRamp()
        if ramp is None:
            return
        choose_ramp(ramp)
        self.show_current()
        self.changed.emit()

    def reset(self) -> None:
        reset_style()
        self.show_current()
        self.changed.emit()
