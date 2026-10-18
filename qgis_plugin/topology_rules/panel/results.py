from __future__ import annotations

import collections.abc as c
import typing as t

from qgis.core import QgsVectorLayer
from qgis.gui import QgisInterface
from qgis.PyQt.QtCore import (
    QCoreApplication,
    QEvent,
    QModelIndex,
    Qt,
    pyqtSignal,
)
from qgis.PyQt.QtGui import (
    QKeyEvent,
    QStandardItemModel,
)
from qgis.PyQt.QtWidgets import (
    QAbstractItemView,
    QAction,
    QCheckBox,
    QPushButton,
    QTreeView,
    QVBoxLayout,
    QWidget,
)

from ..core.style import Grouping, rule_grouping
from ..utils import not_none
from .entries import project_layer
from .focus import Focus
from .hint import Hint
from .ignored import Ignored
from .rows import Row
from .tree import ROLE, errors_under, fill
from .wrapping import wrap_long_text


def tr(text: str) -> str:
    return QCoreApplication.translate('TopologyRules', text)


def quoted(text: str) -> str:
    return "'" + text.replace("'", "''") + "'"


class ResultsPanel(QWidget):
    """The errors of the last run, by rule and kind of error, to zoom to,
    select from and filter the map with."""

    # The user ignored errors, or took them back
    ignored_changed = pyqtSignal()

    def __init__(self, iface: QgisInterface, parent: QWidget | None = None):
        super().__init__(parent)
        self.iface = iface
        self.focus = Focus(iface)
        # The layer of the project that each name in the errors stands for
        self.layers: dict[str, str] = {}
        # All the errors of the run, the ignored ones too
        self.rows: list[Row] = []
        self.ignored = Ignored()
        self.grouping = rule_grouping()
        self.labels: dict[str, str] = {}
        self.model = QStandardItemModel()
        self.model.setHorizontalHeaderLabels([tr('Errors')])
        self.view = QTreeView()
        self.placeholder = Hint(
            self.view, tr('Run the rules to see their errors here.')
        )
        self.view.setModel(self.model)
        self.view.setHeaderHidden(True)
        self.view.setContextMenuPolicy(Qt.ContextMenuPolicy.ActionsContextMenu)
        self.zoom_action = QAction(tr('Zoom to error'), self.view)
        self.select_action = QAction(tr('Select features'), self.view)
        self.ignore_action = QAction(tr('Ignore error'), self.view)
        self.restore_action = QAction(tr('Stop ignoring'), self.view)
        for action in (
            self.zoom_action,
            self.select_action,
            self.ignore_action,
            self.restore_action,
        ):
            self.view.addAction(action)
        self.view.setSelectionMode(
            QAbstractItemView.SelectionMode.SingleSelection
        )
        wrap_long_text(self.view)
        self.view.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self.view.setHorizontalScrollBarPolicy(
            Qt.ScrollBarPolicy.ScrollBarAsNeeded
        )
        self.show_ignored = QCheckBox()
        self.select_button = QPushButton(tr('Select features'))
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.view)
        layout.addWidget(self.show_ignored)
        layout.addWidget(self.select_button)
        not_none(self.view.selectionModel()).selectionChanged.connect(
            self.selection_changed
        )
        self.select_button.clicked.connect(self.select_features)
        self.view.doubleClicked.connect(self.open_error)
        self.view.installEventFilter(self)
        self.select_action.triggered.connect(self.select_features)
        self.zoom_action.triggered.connect(self.zoom)
        self.ignore_action.triggered.connect(self.ignore)
        self.restore_action.triggered.connect(self.restore)
        self.show_ignored.toggled.connect(self.refresh)
        self.show_rows([], {})

    def show_rows(
        self,
        rows: list[Row],
        layers: dict[str, str],
        grouping: Grouping | None = None,
        labels: dict[str, str] | None = None,
    ) -> None:
        """Shows the errors of a run, grouped by the rule of the list that
        found them, in the order of the list, and by kind of error."""
        self.focus.clear()
        self.layers = layers
        self.rows = rows
        self.grouping = grouping or rule_grouping()
        self.labels = labels or {}
        self.show_ignored.setText(
            tr('Show ignored ({count})').format(
                count=sum(self.is_ignored(row) for row in rows)
            )
        )
        fill(
            self.model,
            filter(self.is_shown, rows),
            self.labels,
            self.is_ignored,
        )
        self.placeholder.set_empty(not rows)
        self.update_select()
        self.filter_map(None)

    def clear(self) -> None:
        self.show_rows([], {})

    def refresh(self) -> None:
        """Builds the tree again, after the errors to show have changed."""
        self.show_rows(self.rows, self.layers, self.grouping, self.labels)

    def is_ignored(self, row: Row) -> bool:
        return self.ignored.has(row, self.layers)

    def is_shown(self, row: Row) -> bool:
        return self.show_ignored.isChecked() or not self.is_ignored(row)

    def active(self, key: str | None = None) -> list[Row]:
        """The errors that count, of a group or of all."""
        return [
            row
            for row in self.rows
            if (key is None or row.key == key) and not self.is_ignored(row)
        ]

    def leaves(self) -> list[Row]:
        """The errors of the selected node."""
        return errors_under(self.model.itemFromIndex(self.current()))

    def ignore(self) -> None:
        self.ignored.add(self.leaves(), self.layers)
        self.changed()

    def restore(self) -> None:
        self.ignored.remove(self.leaves(), self.layers)
        self.changed()

    def changed(self) -> None:
        self.refresh()
        self.ignored_changed.emit()

    def error_layers(self) -> list[QgsVectorLayer]:
        found = {row.error_layer for row in self.rows}
        layers = [project_layer(layer_id) for layer_id in found]
        return [layer for layer in layers if layer is not None]

    def filter_map(self, node: tuple[str, str, str | None] | None) -> None:
        """Shows only the errors of a group, or of a kind of error of it, on
        the map, or all of them."""
        parts: list[str] = []
        if node is not None:
            parts.append(f'{self.grouping.expression} = {quoted(node[1])}')
            if node[2] is not None:
                parts.append(f'"error_type" = {quoted(node[2])}')
        hidden: dict[str, list[int]] = {}
        for row in self.rows:
            if not self.is_shown(row):
                hidden.setdefault(row.error_layer, []).append(row.error_fid)
        for layer in self.error_layers():
            fids = sorted(hidden.get(layer.id(), []))
            gone = [f'$id NOT IN ({", ".join(map(str, fids))})'] if fids else []
            expression = ' AND '.join(parts + gone)
            if layer.subsetString() != expression:
                layer.setSubsetString(expression)

    def drop_layers(self, layer_ids: c.Iterable[str]) -> None:
        gone = set(layer_ids)
        if any(row.error_layer in gone for row in self.rows):
            kept = [r for r in self.rows if r.error_layer not in gone]
            self.show_rows(kept, self.layers)

    def current(self) -> QModelIndex:
        selection = self.view.selectionModel()
        indexes = [] if selection is None else selection.selectedIndexes()
        return indexes[0] if indexes else QModelIndex()

    def current_row(self) -> Row | None:
        data = self.current().data(ROLE)
        return data if isinstance(data, Row) else None

    def select_key(self, key: str) -> bool:
        """Selects the group of a rule of the list in the tree."""
        for index in range(self.model.rowCount()):
            item = not_none(self.model.item(index))
            if item.data(ROLE)[1] == key:
                self.view.setCurrentIndex(item.index())
                return True
        return False

    def open_error(self, index: QModelIndex) -> None:
        """A double-click on an error zooms to it and selects its features."""
        if isinstance(index.data(ROLE), Row):
            self.view.setCurrentIndex(index)
            self.zoom()
            self.select_features()

    def eventFilter(self, *args: t.Any) -> bool:  # pyright: ignore[reportIncompatibleMethodOverride]
        """Enter on an error zooms to it again."""
        event = t.cast(QEvent, args[1])
        if event.type() == QEvent.Type.KeyPress and t.cast(
            QKeyEvent, event
        ).key() in (Qt.Key.Key_Return, Qt.Key.Key_Enter):
            self.zoom()
            return True
        return super().eventFilter(*args)

    def update_select(self) -> None:
        enabled = self.current_row() is not None
        self.select_button.setEnabled(enabled)
        self.zoom_action.setEnabled(enabled)
        self.select_action.setEnabled(enabled)
        ignored = [self.is_ignored(row) for row in self.leaves()]
        self.ignore_action.setEnabled(not all(ignored) if ignored else False)
        self.restore_action.setEnabled(any(ignored))

    def zoom(self) -> None:
        """Zooms to the selected error again, after the map was moved."""
        row = self.current_row()
        layer = None if row is None else project_layer(row.error_layer)
        if row is not None and layer is not None and layer.isValid():
            self.focus.show(layer, row.error_fid)

    def selection_changed(self) -> None:
        self.update_select()
        data = self.current().data(ROLE)
        row = data if isinstance(data, Row) else None
        if row is not None:
            self.filter_map(('group', row.key, row.error))
            layer = project_layer(row.error_layer)
            if layer is not None and layer.isValid():
                self.focus.show(layer, row.error_fid)
            return
        self.focus.clear()
        self.filter_map(
            t.cast('tuple[str, str, str | None] | None', data)
            if data is not None
            else None
        )

    def select_features(self) -> None:
        """Selects the features that the selected error is about, replacing
        the selection of their layers."""
        row = self.current_row()
        if row is None:
            return
        chosen: dict[str, list[int]] = {}
        for alias, fid in (
            (row.layer_alias, row.feature),
            (row.other_alias, row.other_feature),
        ):
            layer_id = self.layers.get(alias)
            if layer_id is not None and fid is not None:
                chosen.setdefault(layer_id, []).append(fid)
        for layer_id, fids in chosen.items():
            layer = project_layer(layer_id)
            if layer is not None:
                layer.selectByIds(fids)
