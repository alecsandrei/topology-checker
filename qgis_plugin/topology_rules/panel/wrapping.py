from __future__ import annotations

import typing as t

from qgis.PyQt.QtCore import QModelIndex, QSize, Qt, QTimer
from qgis.PyQt.QtGui import QFont, QTextLayout, QTextOption
from qgis.PyQt.QtWidgets import (
    QApplication,
    QStyle,
    QStyledItemDelegate,
    QStyleOptionViewItem,
    QTreeView,
)

from ..utils import not_none


def line_count(text: str, font: QFont, width: int) -> int:
    """How many lines the text takes when wrapped at the width, the way the
    style wraps the text of an item: at a space, else anywhere."""
    layout = QTextLayout(text, font)
    wrapping = QTextOption()
    wrapping.setWrapMode(QTextOption.WrapMode.WrapAtWordBoundaryOrAnywhere)
    layout.setTextOption(wrapping)
    layout.beginLayout()
    while (line := layout.createLine()).isValid():
        line.setLineWidth(width)
    layout.endLayout()
    return layout.lineCount()


def icon_size(view: QTreeView) -> QSize:
    """The size of the icons of the view. The option that a view gives to a
    delegate holds no size for them in Qt6."""
    size = view.iconSize()
    if size.isValid():
        return size
    side = not_none(QApplication.style()).pixelMetric(
        QStyle.PixelMetric.PM_SmallIconSize
    )
    return QSize(side, side)


class WrapDelegate(QStyledItemDelegate):
    """Gives an item in the first column the height of its text wrapped at
    the width of the column, so a long text takes a second line and a short
    one stays one line."""

    def __init__(self, view: QTreeView):
        super().__init__(view)
        self.view = view

    def sizeHint(  # pyright: ignore[reportIncompatibleMethodOverride]
        self, *args: t.Any
    ) -> QSize:
        option = t.cast(QStyleOptionViewItem, args[0])
        index = t.cast(QModelIndex, args[1])
        # The height of one line: the style wraps the text itself in Qt6, at
        # a width that is not the one of the column
        single = QStyleOptionViewItem(option)
        single.features &= ~QStyleOptionViewItem.ViewItemFeature.WrapText
        size = super().sizeHint(single, index)
        # Rows with an icon and rows without are as high as one another
        icon = icon_size(self.view)
        size.setHeight(max(size.height(), icon.height() + 6))
        text = str(index.data() or '')
        if index.column() != 0 or not text:
            return size
        # What is left of the column after the branches, the check box and
        # the icon
        depth, parent = 1, index.parent()
        while parent.isValid():
            depth, parent = depth + 1, parent.parent()
        width = self.view.columnWidth(0) - depth * self.view.indentation()
        style = not_none(QApplication.style())
        if index.data(Qt.ItemDataRole.CheckStateRole) is not None:
            width -= style.pixelMetric(QStyle.PixelMetric.PM_IndicatorWidth)
            width -= style.pixelMetric(
                QStyle.PixelMetric.PM_CheckBoxLabelSpacing
            )
        if index.data(Qt.ItemDataRole.DecorationRole) is not None:
            width -= icon.width() + 4
        margin = style.pixelMetric(QStyle.PixelMetric.PM_FocusFrameHMargin) + 1
        lines = line_count(text, option.font, max(width - 2 * margin, 20))
        # The height of one line is what the style gave; the others add theirs
        spacing = option.fontMetrics.lineSpacing()
        # A pixel more for the lines under the first, which the descent of a
        # font that is a little taller than its line spacing would clip
        extra = (lines - 1) * spacing + (1 if lines > 1 else 0)
        size.setHeight(size.height() + extra)
        return size


def relayout(view: QTreeView, delegate: WrapDelegate) -> None:
    """Measures the rows that show again, which the view does not do by
    itself for the children of an item when the column changes. Taller rows
    can bring a scroll bar, which narrows the column, so it goes on until the
    width holds."""
    model = not_none(view.model())
    for _ in range(3):
        width = view.columnWidth(0)
        pending = [QModelIndex()]
        while pending:
            parent = pending.pop()
            for row in range(model.rowCount(parent)):
                index = model.index(row, 0, parent)
                delegate.sizeHintChanged.emit(index)
                if view.isExpanded(index):
                    pending.append(index)
        t.cast(t.Any, view).doItemsLayout()
        if view.columnWidth(0) == width:
            break


def wrap_long_text(view: QTreeView) -> None:
    """Makes the view wrap the long texts of its first column, and lay its
    rows out again when the column gets wider or narrower."""
    view.setWordWrap(True)
    view.setTextElideMode(Qt.TextElideMode.ElideNone)
    view.setUniformRowHeights(False)
    delegate = WrapDelegate(view)
    view.setItemDelegate(delegate)
    later = QTimer(view)
    later.setSingleShot(True)
    # The next turn of the event loop, once the sections have settled
    later.setInterval(0)
    later.timeout.connect(lambda: relayout(view, delegate))
    not_none(view.header()).sectionResized.connect(lambda *_: later.start())
