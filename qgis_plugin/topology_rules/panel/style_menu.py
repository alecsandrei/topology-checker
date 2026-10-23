from __future__ import annotations

import typing as t

from qgis.core import QgsColorRamp, QgsStyle, QgsSymbolLayerUtils
from qgis.gui import QgsColorRampButton
from qgis.PyQt.QtCore import QSize, pyqtSignal
from qgis.PyQt.QtGui import QIcon
from qgis.PyQt.QtWidgets import QAction, QActionGroup, QMenu, QWidget

from ..core.style import (
    ColourBy,
    Size,
    choose,
    choose_ramp,
    colour_by,
    colour_ramp,
    default_ramp,
    reset_style,
    same_ramp,
    size,
)
from ..enums import Setting
from ..utils import not_none, tr

ICON_SIZE = QSize(16, 16)


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
        # The default ramp and the favourite ones of the style, then all of
        # them, filled when the menu opens, as the style can have changed
        self.ramp_menu = not_none(self.addMenu(tr('Colour ramp')))
        self.ramps = QActionGroup(self.ramp_menu)
        self.all_menu = QMenu(tr('All ramps'), self.ramp_menu)
        self.all_ramps = QActionGroup(self.all_menu)
        self.invert_action = QAction(tr('Invert'), self)
        self.edit_action = QAction(tr('Edit…'), self)
        # Never shown: a click on it opens the editor of QGIS for the kind of
        # the ramp, and it says when the ramp was changed there
        self.ramp_button = QgsColorRampButton(self, tr('Colour ramp'))
        self.ramp_button.hide()
        self.addSeparator()
        self.reset_action = not_none(self.addAction(tr('Reset style')))

        self.colours.triggered.connect(self.colour_chosen)
        self.sizes.triggered.connect(self.size_chosen)
        for group in (self.ramps, self.all_ramps):
            group.triggered.connect(self.named_ramp_chosen)
        self.invert_action.triggered.connect(self.invert)
        self.edit_action.triggered.connect(self.edit)
        self.ramp_button.colorRampChanged.connect(self.ramp_edited)
        self.reset_action.triggered.connect(self.reset)
        self.aboutToShow.connect(self.show_current)
        self.ramp_menu.aboutToShow.connect(self.fill_ramps)
        self.show_current()
        self.fill_ramps()

    def show_current(self) -> None:
        """Checks what the user chose, which can have changed elsewhere."""
        for group, value in (
            (self.colours, colour_by()),
            (self.sizes, size()),
        ):
            for action in group.actions():
                action.setChecked(action.data() == value.value)
        self.ramp_menu.setIcon(ramp_icon(colour_ramp()))

    def fill_ramps(self) -> None:
        """The ramps to choose from, with the one in use checked. The
        menus delete their items, which leave the groups."""
        self.ramp_menu.clear()
        self.all_menu.clear()
        style = not_none(QgsStyle.defaultStyle())
        entity = QgsStyle.StyleEntity.ColorrampEntity
        current = colour_ramp()
        named = [('', tr('Default'), default_ramp())]
        named += [
            (name, name, ramp)
            for name in style.symbolsOfFavorite(entity)
            if (ramp := style.colorRamp(name)) is not None
        ]
        everything = [
            (name, name, ramp)
            for name in style.colorRampNames()
            if (ramp := style.colorRamp(name)) is not None
        ]
        for menu, group, ramps in (
            (self.ramp_menu, self.ramps, named),
            (self.all_menu, self.all_ramps, everything),
        ):
            for name, text, ramp in ramps:
                action = not_none(menu.addAction(ramp_icon(ramp), text))
                action.setCheckable(True)
                action.setData(name)
                action.setChecked(same_ramp(ramp, current))
                group.addAction(action)
        self.ramp_menu.addMenu(self.all_menu)
        self.ramp_menu.addSeparator()
        self.ramp_menu.addAction(self.invert_action)
        self.ramp_menu.addAction(self.edit_action)

    def colour_chosen(self) -> None:
        action = not_none(self.colours.checkedAction())
        value = ColourBy(action.data())
        if value is not colour_by():
            choose(Setting.COLOUR_BY, value)
            self.changed.emit()

    def size_chosen(self) -> None:
        action = not_none(self.sizes.checkedAction())
        value = Size(action.data())
        if value is not size():
            choose(Setting.SIZE, value)
            self.changed.emit()

    def named_ramp_chosen(self, action: t.Any) -> None:
        name = str(action.data() or '')
        if not name:
            self.use_ramp(default_ramp())
        elif ramp := not_none(QgsStyle.defaultStyle()).colorRamp(name):
            self.use_ramp(ramp)

    def invert(self) -> None:
        ramp = colour_ramp()
        ramp.invert()
        self.use_ramp(ramp)

    def edit(self) -> None:
        """Opens the editor of QGIS for the ramp in use."""
        self.ramp_button.blockSignals(True)
        self.ramp_button.setColorRamp(colour_ramp())
        self.ramp_button.blockSignals(False)
        self.ramp_button.click()

    def ramp_edited(self) -> None:
        ramp = self.ramp_button.colorRamp()
        if ramp is not None:
            self.use_ramp(ramp)

    def use_ramp(self, ramp: QgsColorRamp) -> None:
        """Draws the errors with the ramp, unless they already are."""
        if same_ramp(ramp, colour_ramp()):
            return
        choose_ramp(ramp)
        self.show_current()
        self.changed.emit()

    def reset(self) -> None:
        reset_style()
        self.show_current()
        self.changed.emit()


def ramp_icon(ramp: QgsColorRamp) -> QIcon:
    """A small picture of the colours of a ramp."""
    return QgsSymbolLayerUtils.colorRampPreviewIcon(ramp, ICON_SIZE)
