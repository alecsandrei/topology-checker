from __future__ import annotations

import collections.abc as c
import typing as t

from qgis.core import QgsCoordinateReferenceSystem, QgsVectorLayer
from qgis.gui import QgisInterface
from qgis.PyQt.QtCore import (
    QEvent,
    QModelIndex,
    QSignalBlocker,
    Qt,
    pyqtSignal,
)
from qgis.PyQt.QtGui import QKeyEvent, QStandardItem
from qgis.PyQt.QtWidgets import (
    QAbstractItemView,
    QAction,
    QCheckBox,
    QPushButton,
    QTreeView,
    QVBoxLayout,
    QWidget,
)

from ..core.errors import from_wkb
from ..core.style import Grouping, rule_grouping
from ..utils import not_none, tr
from .entries import project_layer
from .focus import Focus
from .hint import Hint
from .ignored import Ignored, stamp
from .rows import Row, Shapes
from .tree import (
    ROLE,
    ErrorModel,
    fill,
    find_group,
    group_key,
    grouped,
    put_group,
)
from .wrapping import wrap_long_text

Node = tuple[str, str, str | None]


def quoted(text: str) -> str:
    return "'" + text.replace("'", "''") + "'"


def idle_text() -> str:
    return tr('Run the rules to see their errors here.')


def node_of(data: object) -> Node | None:
    """The group or kind that an item of the tree is, or is in."""
    if isinstance(data, Row):
        return ('group', data.key, data.error)
    return t.cast('Node', data) if data else None


def kind_of(item: QStandardItem) -> str | None:
    return t.cast('Node', item.data(ROLE))[2]


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
        # The error layer of each family of the errors of the run
        self.error_layer_ids: dict[str, str] = {}
        self.ignored = Ignored()
        self.grouping = rule_grouping()
        self.labels: dict[str, str] = {}
        # The geometries of the errors of each group by family while they
        # have no layer, during a run, in the CRS of the run
        self.shapes: dict[str, dict[str, Shapes]] = {}
        self.crs = QgsCoordinateReferenceSystem()
        self.model = ErrorModel()
        self.model.setHorizontalHeaderLabels([tr('Errors')])
        self.view = QTreeView()
        self.placeholder = Hint(self.view, idle_text())
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
        self.rows = stamp(rows, layers)
        rows = self.rows
        self.grouping = grouping or rule_grouping()
        self.labels = labels or {}
        self.count_ignored()
        fill(
            self.model,
            filter(self.is_shown, rows),
            self.labels,
            self.is_ignored,
        )
        self.placeholder.set_empty(not rows)
        self.update_select()
        self.filter_map(None)

    def count_ignored(self) -> None:
        self.show_ignored.setText(
            tr('Show ignored ({count})').format(
                count=sum(self.is_ignored(row) for row in self.rows)
            )
        )

    def clear(self, labels: dict[str, str] | None = None) -> None:
        self.error_layer_ids = {}
        self.shapes = {}
        self.show_rows([], {}, None, labels)

    def begin(
        self, labels: dict[str, str], crs: QgsCoordinateReferenceSystem
    ) -> None:
        """Starts the tree over for a run whose errors come rule by rule, in
        the CRS of the run."""
        self.clear(labels)
        self.crs = crs
        self.placeholder.setText(tr('No errors yet.'))

    def end(self) -> None:
        self.placeholder.setText(idle_text())

    def add_rule(
        self,
        key: str,
        rows: list[Row],
        layers: c.Mapping[str, str],
        shapes: dict[str, Shapes],
    ) -> None:
        """Shows the errors of a rule that is done where the list has the
        rule, or the features that were left out of the run last. The other
        groups stay as they are."""
        self.layers = {**self.layers, **layers}
        self.rows = [row for row in self.rows if row.key != key] + rows
        self.shapes[key] = shapes
        put_group(
            self.model,
            key,
            filter(self.is_shown, rows),
            self.labels,
            self.is_ignored,
        )
        self.count_ignored()
        self.placeholder.set_empty(not self.rows)
        self.update_select()

    def drop_shapes(self) -> None:
        """Lets go of the geometries of the errors of the run, which their
        layers hold from now on."""
        self.shapes = {}

    def hand_over(
        self,
        layer_ids: c.Mapping[str, str],
        layers: dict[str, str],
        grouping: Grouping,
    ) -> None:
        """The errors of the run are in their layers now, each with the id
        it was told: the map shows them as the tree does. The tree stays as
        it is, open, selected and scrolled as the user left it."""
        self.error_layer_ids = dict(layer_ids)
        self.layers = layers
        self.grouping = grouping
        self.filter_map(node_of(self.current().data(ROLE)))

    def rebuild(self, key: str, rows: list[Row]) -> None:
        """Builds a group again, as open as it was, with the same item
        current if it still has it."""
        index = find_group(self.model, key)
        opened = False
        kinds_open: set[str | None] = set()
        current: tuple[str | None, int] | None = None
        if index >= 0:
            group = not_none(self.model.item(index))
            opened = self.view.isExpanded(group.index())
            for i in range(group.rowCount()):
                child = not_none(group.child(i))
                if self.view.isExpanded(child.index()):
                    kinds_open.add(kind_of(child))
            path: list[int] = []
            here = self.view.currentIndex()
            while here.isValid():
                path.insert(0, here.row())
                here = here.parent()
            if path and path[0] == index:
                kind = (
                    kind_of(not_none(group.child(path[1])))
                    if len(path) > 1
                    else None
                )
                current = (kind, path[2] if len(path) > 2 else -1)
        selection = not_none(self.view.selectionModel())
        blocker = QSignalBlocker(selection)
        try:
            item = put_group(
                self.model, key, rows, self.labels, self.is_ignored
            )
            if item is None:
                return
            self.view.setExpanded(item.index(), opened)
            if current is not None and current[0] is None:
                self.view.setCurrentIndex(item.index())
            for i in range(item.rowCount()):
                child = not_none(item.child(i))
                kind = kind_of(child)
                if kind in kinds_open:
                    self.view.setExpanded(child.index(), True)
                if current is not None and current[0] == kind:
                    leaf = child.child(current[1]) if current[1] >= 0 else None
                    self.view.setCurrentIndex((leaf or child).index())
        finally:
            blocker.unblock()
        not_none(self.view.viewport()).update()

    def refresh(self) -> None:
        """Builds the groups again, after the errors to show have changed.
        The tree stays as open and scrolled as it was. Nothing is selected
        then, and the map shows all the errors that are shown."""
        bar = not_none(self.view.verticalScrollBar())
        scrolled = bar.value()
        shown = grouped(filter(self.is_shown, self.rows), list(self.labels))
        for index in reversed(range(self.model.rowCount())):
            key = group_key(not_none(self.model.item(index)))
            if key not in shown:
                self.rebuild(key, [])
        for key, kinds in shown.items():
            self.rebuild(key, [row for rows in kinds.values() for row in rows])
        selection = not_none(self.view.selectionModel())
        blocker = QSignalBlocker(selection)
        try:
            selection.clear()
        finally:
            blocker.unblock()
        not_none(self.view.viewport()).update()
        bar.setValue(scrolled)
        self.count_ignored()
        self.placeholder.set_empty(not self.rows)
        self.selection_changed()

    def is_ignored(self, row: Row) -> bool:
        return self.ignored.has(row)

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
        return self.model.errors_of(self.current())

    def ignore(self) -> None:
        self.ignored.add(self.leaves())
        self.changed()

    def restore(self) -> None:
        self.ignored.remove(self.leaves())
        self.changed()

    def changed(self) -> None:
        self.refresh()
        self.ignored_changed.emit()

    def error_layer(self, row: Row) -> QgsVectorLayer | None:
        layer_id = self.error_layer_ids.get(row.family)
        return None if layer_id is None else project_layer(layer_id)

    def error_layers(self) -> dict[str, QgsVectorLayer]:
        """The error layers of the run, by family."""
        layers = {
            family: project_layer(layer_id)
            for family, layer_id in self.error_layer_ids.items()
        }
        return {
            family: layer
            for family, layer in layers.items()
            if layer is not None
        }

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
                hidden.setdefault(row.family, []).append(row.error_fid)
        for family, layer in self.error_layers().items():
            fids = sorted(hidden.get(family, []))
            gone = [f'$id NOT IN ({", ".join(map(str, fids))})'] if fids else []
            expression = ' AND '.join(parts + gone)
            if layer.subsetString() != expression:
                layer.setSubsetString(expression)

    def drop_layers(self, layer_ids: c.Iterable[str]) -> None:
        """Drops the errors of the error layers that left the project."""
        gone = set(layer_ids)
        families = {
            family
            for family, layer_id in self.error_layer_ids.items()
            if layer_id in gone
        }
        if not families:
            return
        for family in families:
            del self.error_layer_ids[family]
        kept = [row for row in self.rows if row.family not in families]
        self.show_rows(kept, self.layers, self.grouping, self.labels)

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
        if row is not None:
            self.show_error(row)

    def show_error(self, row: Row) -> None:
        """Zooms to the error in its layer, or to its own geometry while it
        has no layer."""
        layer = self.error_layer(row)
        if layer is not None:
            if layer.isValid():
                self.focus.show(layer, row.error_fid)
            return
        wkb = self.shape_of(row)
        if wkb:
            self.focus.show_geometry(from_wkb(wkb), self.crs)

    def shape_of(self, row: Row) -> bytes:
        """The geometry of an error that has no layer, as WKB, if it has
        one."""
        shapes = self.shapes.get(row.key, {}).get(row.family)
        return b'' if shapes is None else shapes.of(row.error_fid)

    def selection_changed(self) -> None:
        self.update_select()
        data = self.current().data(ROLE)
        row = data if isinstance(data, Row) else None
        if row is not None:
            self.filter_map(('group', row.key, row.error))
            self.show_error(row)
            return
        self.focus.clear()
        self.filter_map(node_of(data))

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
