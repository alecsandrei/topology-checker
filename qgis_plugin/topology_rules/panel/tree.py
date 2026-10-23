from __future__ import annotations

import collections.abc as c
import typing as t

from qgis.PyQt.QtCore import QModelIndex, Qt
from qgis.PyQt.QtGui import QBrush, QColor, QStandardItem, QStandardItemModel

from .rows import Row, humanize, plural

# What an item of the tree holds: an error, or ('group', rule key, kind)
ROLE = Qt.ItemDataRole.UserRole


def rank(key: str, order: list[str]) -> int:
    """Where a group goes: as the list has it, and the errors of features
    that were left out last."""
    if not key:
        return len(order) + 1
    return order.index(key) if key in order else len(order)


class ErrorModel(QStandardItemModel):
    """The errors, by group and kind of error, with the errors of each kind
    at hand, so that a group of thousands is not read item by item."""

    def __init__(self) -> None:
        super().__init__()
        # The errors of each kind, by rule key and kind, in their order
        self.kinds: dict[tuple[str, str], list[Row]] = {}

    def errors_of(self, index: QModelIndex) -> list[Row]:
        """The errors of a node: itself, or all of its errors."""
        data = index.data(ROLE)
        if isinstance(data, Row):
            return [data]
        if not data:
            return []
        node = t.cast('tuple[str, str, str | None]', data)
        if node[2] is not None:
            return list(self.kinds.get((node[1], node[2]), []))
        return [
            row
            for (key, _), rows in self.kinds.items()
            if key == node[1]
            for row in rows
        ]


def fill(
    model: ErrorModel,
    rows: c.Iterable[Row],
    labels: c.Mapping[str, str],
    ignored: c.Callable[[Row], bool],
) -> None:
    """Replaces the items of the model with the errors, grouped by the rule
    of the list that found them, in the order of the list, and by kind of
    error. The errors that are ignored are grey."""
    model.removeRows(0, model.rowCount())
    model.kinds = {}
    order = list(labels)
    groups: dict[str, dict[str, list[Row]]] = {}
    for row in sorted(rows, key=lambda r: (rank(r.key, order), r.group)):
        groups.setdefault(row.key, {}).setdefault(row.error, []).append(row)
    grey = QBrush(QColor(Qt.GlobalColor.gray))
    for key, kinds in groups.items():
        first = next(iter(next(iter(kinds.values()))))
        name = labels.get(key) or first.group
        group_item = QStandardItem(
            f'{name} ({sum(len(r) for r in kinds.values())})'
        )
        group_item.setData(('group', key, None), ROLE)
        for error, group in sorted(kinds.items()):
            kind_item = QStandardItem(f'{plural(error)} ({len(group)})')
            kind_item.setData(('group', key, error), ROLE)
            ordered = sorted(group, key=lambda r: (r.layer, r.feature or -1))
            model.kinds[key, error] = ordered
            items: list[QStandardItem] = []
            for number, row in enumerate(ordered, start=1):
                item = QStandardItem(
                    row.features or f'{humanize(error)} {number}'
                )
                item.setData(row, ROLE)
                if ignored(row):
                    item.setForeground(grey)
                items.append(item)
            # At once, before the kind is in the model: one item at a time
            # tells the view of each of them
            kind_item.appendRows(items)
            group_item.appendRow(kind_item)
        model.appendRow(group_item)
