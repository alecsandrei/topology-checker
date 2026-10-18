from __future__ import annotations

import contextlib
import typing as t

from qgis.core import QgsIconUtils
from qgis.PyQt.QtCore import Qt
from qgis.PyQt.QtGui import QIcon
from qgis.PyQt.QtWidgets import (
    QApplication,
    QStyle,
    QTreeWidget,
    QTreeWidgetItem,
)

from ..utils import not_none, tr
from .entries import Entry, project_layer, where_is

# What an item of the list holds: the index of its entry (a rule), the id of
# its layer (a layer), the id of its other layer (a rule)
ENTRY = Qt.ItemDataRole.UserRole
LAYER = Qt.ItemDataRole.UserRole + 1
OTHER = Qt.ItemDataRole.UserRole + 2


def warning_icon() -> QIcon:
    return not_none(QApplication.style()).standardIcon(
        QStyle.StandardPixmap.SP_MessageBoxWarning
    )


def tip(entry: Entry) -> str:
    """The whole name of the rule, and what it checks."""
    return f'{entry.rule.display_name}\n{entry.rule.description}'


def layer_text(
    entries: list[Entry], invalid: dict[str, int], node: QTreeWidgetItem
) -> str:
    """The name of a layer, with how many of its features were left out as
    invalid."""
    layer_id = node.data(0, LAYER)
    name = next((e.layer_name for e in entries if e.layer == layer_id), '')
    left_out = invalid.get(layer_id, 0)
    if not left_out:
        return name
    return f'{name} · ' + tr('{count} invalid').format(count=left_out)


def mark_nodes(
    tree: QTreeWidget,
    entries: list[Entry],
    invalid: dict[str, int],
    names_changed: t.Callable[[], None],
    crs_changed: t.Callable[[], None],
) -> None:
    """Marks the layers that are not in the project or have invalid features,
    and the rules that take a layer that is not there as the other layer.
    Listens to the layers for a new name or CRS."""
    missing = tr('This layer is not in the project.')
    for index in range(tree.topLevelItemCount()):
        node = not_none(tree.topLevelItem(index))
        layer = project_layer(node.data(0, LAYER))
        node.setIcon(
            0,
            warning_icon()
            if layer is None
            else QgsIconUtils.iconForLayer(layer),
        )
        node.setToolTip(0, missing if layer is None else where_is(layer))
        left_out = invalid.get(node.data(0, LAYER), 0)
        node.setText(0, layer_text(entries, invalid, node))
        if left_out and layer is not None:
            node.setIcon(0, warning_icon())
            node.setToolTip(
                0,
                where_is(layer)
                + '\n'
                + tr(
                    '{count} features have an invalid geometry and were '
                    'left out of the run.'
                ).format(count=left_out),
            )
        if layer is not None:
            for signal, slot in (
                (layer.nameChanged, names_changed),
                (layer.crsChanged, crs_changed),
            ):
                # A connection that is there already raises
                with contextlib.suppress(TypeError):
                    t.cast(t.Any, signal).connect(
                        slot, Qt.ConnectionType.UniqueConnection
                    )
        for row in range(node.childCount()):
            child = not_none(node.child(row))
            other = child.data(0, OTHER)
            gone = other is not None and project_layer(other) is None
            child.setIcon(0, warning_icon() if gone else QIcon())
            child.setToolTip(
                0,
                missing if gone else tip(entries[child.data(0, ENTRY)]),
            )
