from __future__ import annotations

import collections.abc as c

from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtGui import QBrush, QColor, QStandardItem, QStandardItemModel

from ..utils import not_none
from .rows import Row, humanize, plural

# What an item of the tree holds: an error, or ('group', rule key, kind)
ROLE = Qt.ItemDataRole.UserRole


def rank(key: str, order: list[str]) -> int:
    """Where a group goes: as the list has it, and the errors of features
    that were left out last."""
    if not key:
        return len(order) + 1
    return order.index(key) if key in order else len(order)


def fill(
    model: QStandardItemModel,
    rows: c.Iterable[Row],
    labels: c.Mapping[str, str],
    ignored: c.Callable[[Row], bool],
) -> None:
    """Replaces the items of the model with the errors, grouped by the rule
    of the list that found them, in the order of the list, and by kind of
    error. The errors that are ignored are grey."""
    model.removeRows(0, model.rowCount())
    order = list(labels)
    groups: dict[str, dict[str, list[Row]]] = {}
    for row in sorted(rows, key=lambda r: (rank(r.key, order), r.group)):
        groups.setdefault(row.key, {}).setdefault(row.error, []).append(row)
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
            for number, row in enumerate(ordered, start=1):
                item = QStandardItem(
                    row.features or f'{humanize(error)} {number}'
                )
                item.setData(row, ROLE)
                if ignored(row):
                    item.setForeground(QBrush(QColor(Qt.GlobalColor.gray)))
                kind_item.appendRow(item)
            group_item.appendRow(kind_item)
        model.appendRow(group_item)


def errors_under(item: QStandardItem | None) -> list[Row]:
    """The errors of a node: itself, or all of its errors."""
    stack = [] if item is None else [item]
    found: list[Row] = []
    while stack:
        item = stack.pop()
        data = item.data(ROLE)
        if isinstance(data, Row):
            found.append(data)
        stack.extend(not_none(item.child(i)) for i in range(item.rowCount()))
    return found
