from __future__ import annotations

import typing as t

from qgis.PyQt.QtCore import QEvent, Qt, pyqtSignal
from qgis.PyQt.QtGui import QKeyEvent
from qgis.PyQt.QtWidgets import (
    QAbstractItemView,
    QHBoxLayout,
    QHeaderView,
    QPushButton,
    QTreeWidget,
    QTreeWidgetItem,
    QVBoxLayout,
    QWidget,
)

from ..utils import not_none
from .entries import Entry, project_layer, unit_of
from .hint import Hint
from .icons import decorate
from .marks import ENTRY, LAYER, OTHER, layer_text, mark_nodes, tip, tr
from .narrow import shrink_buttons
from .wrapping import wrap_long_text

COLUMNS = ('Rule', 'Errors')


class RuleList(QWidget):
    """The rules that a run will check, under the layers they check, with
    the errors of the last run."""

    changed = pyqtSignal()
    # A rule was double-clicked
    opened = pyqtSignal(object)

    def __init__(self, parent: QWidget | None = None):
        super().__init__(parent)
        self.entries: list[Entry] = []
        # How many features of each layer a run left out as invalid
        self.invalid: dict[str, int] = {}
        self.filling = False
        self.tree = QTreeWidget()
        self.tree.setColumnCount(len(COLUMNS))
        self.tree.setHeaderLabels(['', ''])
        self.tree.setSelectionMode(
            QAbstractItemView.SelectionMode.ExtendedSelection
        )
        self.tree.setHeaderHidden(True)
        self.tree.setHorizontalScrollBarPolicy(
            Qt.ScrollBarPolicy.ScrollBarAlwaysOff
        )
        self.tree.setIndentation(6)
        header = not_none(self.tree.header())
        header.setStretchLastSection(False)
        header.setMinimumSectionSize(8)
        header.setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        # Room for a number of four digits, whatever the font, and no more
        header.setSectionResizeMode(1, QHeaderView.ResizeMode.Fixed)
        header.resizeSection(
            1, self.tree.fontMetrics().horizontalAdvance('0000') + 2
        )
        wrap_long_text(self.tree)
        self.placeholder = Hint(
            self.tree, tr('Choose a layer and a rule above, then add the rule.')
        )
        self.remove_button = QPushButton(tr('Remove'))
        self.clear_button = QPushButton(tr('Clear'))
        decorate(
            self.remove_button,
            'remove',
            tr('Take the selected rules off the list'),
        )
        decorate(
            self.clear_button, 'clear', tr('Take all the rules off the list')
        )
        shrink_buttons(self.remove_button, self.clear_button)
        buttons = QHBoxLayout()
        buttons.addWidget(self.remove_button)
        buttons.addWidget(self.clear_button)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.tree)
        layout.addLayout(buttons)
        self.tree.installEventFilter(self)
        self.remove_button.clicked.connect(self.remove_selected)
        self.clear_button.clicked.connect(self.clear)
        self.tree.itemSelectionChanged.connect(self.update_buttons)
        self.tree.itemChanged.connect(self.item_changed)
        self.tree.itemDoubleClicked.connect(self.item_opened)
        self.show_entries()

    def add(self, entry: Entry) -> bool:
        """Adds the entry, unless the same rule is there on the same layers."""
        if any(known.key == entry.key for known in self.entries):
            return False
        self.entries.append(entry)
        self.show_entries()
        return True

    def selected_entries(self) -> list[Entry]:
        chosen: list[Entry] = []
        for item in self.tree.selectedItems():
            children = (
                [item]
                if item.parent()
                else [not_none(item.child(i)) for i in range(item.childCount())]
            )
            for child in children:
                index = child.data(0, ENTRY)
                if index is not None and self.entries[index] not in chosen:
                    chosen.append(self.entries[index])
        return chosen

    def remove_selected(self) -> None:
        gone = self.selected_entries()
        self.entries = [e for e in self.entries if e not in gone]
        self.show_entries()

    def clear(self) -> None:
        self.entries = []
        self.show_entries()

    def set_invalid(self, counts: dict[str, int]) -> None:
        self.invalid = {i: n for i, n in counts.items() if n}
        self.refresh_marks()

    def replace(self, entries: list[Entry]) -> None:
        self.entries = entries
        self.show_entries()

    def update_buttons(self) -> None:
        self.remove_button.setEnabled(bool(self.tree.selectedItems()))
        self.clear_button.setEnabled(bool(self.entries))

    def item_changed(self, item: QTreeWidgetItem, column: int) -> None:
        index = item.data(0, ENTRY)
        if self.filling or column != 0 or index is None:
            return
        checked = item.checkState(0) == Qt.CheckState.Checked
        # An icon or a tooltip that changes is not a change of the list
        if self.entries[index].checked != checked:
            self.entries[index].checked = checked
            self.changed.emit()

    def item_opened(self, item: QTreeWidgetItem, column: int) -> None:
        index = item.data(0, ENTRY)
        if index is not None:
            self.opened.emit(self.entries[index])

    def crs_changed(self) -> None:
        """Takes the unit of the new CRS of a layer, for the tolerance. The
        list and the results are what the last run found, and stay."""
        for entry in self.entries:
            layer = project_layer(entry.layer)
            if layer is not None:
                entry.unit = unit_of(layer)
        self.changed.emit()

    def names_changed(self) -> None:
        """Shows the new name of a layer that was renamed."""
        self.filling = True
        try:
            self.rename_nodes()
        finally:
            self.filling = False
        # The project keeps the names
        self.changed.emit()

    def rename_nodes(self) -> None:
        for entry in self.entries:
            layer = project_layer(entry.layer)
            if layer is not None:
                entry.layer_name = layer.name()
            other = project_layer(entry.other)
            if other is not None:
                entry.other_name = other.name()
        for index in range(self.tree.topLevelItemCount()):
            node = not_none(self.tree.topLevelItem(index))
            for entry in self.entries:
                if entry.layer == node.data(0, LAYER):
                    node.setText(
                        0, layer_text(self.entries, self.invalid, node)
                    )
                    break
            for row in range(node.childCount()):
                child = not_none(node.child(row))
                child.setText(0, self.entries[child.data(0, ENTRY)].title)

    def eventFilter(self, *args: t.Any) -> bool:  # pyright: ignore[reportIncompatibleMethodOverride]
        """Delete and Backspace remove the selected rules."""
        event = t.cast(QEvent, args[1])
        if event.type() == QEvent.Type.KeyPress and t.cast(
            QKeyEvent, event
        ).key() in (Qt.Key.Key_Delete, Qt.Key.Key_Backspace):
            self.remove_selected()
            return True
        return super().eventFilter(*args)

    def refresh_marks(self) -> None:
        """Marks the layers that are not in the project, and the rules that
        take one of them as the other layer, where they are."""
        was = self.filling
        self.filling = True
        try:
            mark_nodes(
                self.tree,
                self.entries,
                self.invalid,
                self.names_changed,
                self.crs_changed,
            )
        finally:
            self.filling = was

    def set_count(self, entry: Entry) -> None:
        """Shows the errors of one rule, as they are known, and leaves the
        rest of the list as it is."""
        index = next(
            (i for i, other in enumerate(self.entries) if other is entry), None
        )
        if index is None:
            return
        was = self.filling
        self.filling = True
        try:
            for top in range(self.tree.topLevelItemCount()):
                node = not_none(self.tree.topLevelItem(top))
                for row in range(node.childCount()):
                    child = not_none(node.child(row))
                    if child.data(0, ENTRY) == index:
                        child.setText(
                            1, '' if entry.errors is None else str(entry.errors)
                        )
        finally:
            self.filling = was

    def show_entries(self) -> None:
        """Fills the tree from the entries, marking the layers that have left
        the project."""
        self.filling = True
        self.tree.clear()
        parents: dict[str, QTreeWidgetItem] = {}
        for index, entry in enumerate(self.entries):
            parent = parents.get(entry.layer)
            if parent is None:
                parent = QTreeWidgetItem([entry.layer_name])
                parent.setData(0, LAYER, entry.layer)
                self.tree.addTopLevelItem(parent)
                parents[entry.layer] = parent
            child = QTreeWidgetItem(
                [entry.title, '' if entry.errors is None else str(entry.errors)]
            )
            child.setData(0, ENTRY, index)
            child.setFlags(child.flags() | Qt.ItemFlag.ItemIsUserCheckable)
            child.setCheckState(
                0,
                Qt.CheckState.Checked
                if entry.checked
                else Qt.CheckState.Unchecked,
            )
            child.setData(0, OTHER, entry.other)
            child.setToolTip(0, tip(entry))
            child.setTextAlignment(1, Qt.AlignmentFlag.AlignRight)
            child.setToolTip(1, tr('Errors found in the last run'))
            parent.addChild(child)
        self.refresh_marks()
        self.tree.expandAll()
        self.placeholder.set_empty(not self.entries)
        self.filling = False
        self.update_buttons()
        self.changed.emit()
