"""Long names in the narrow dock: the lists wrap them onto a second line,
and never cut them short with an ellipsis.

How much room a text has depends on the style, which differs between QGIS
versions and systems. So the room is worked out from the metrics of the style,
and the tests only judge what is clear with a few pixels to spare: a text that
fits even in less room must stay on one line, one that does not fit even in
more room must wrap."""

from __future__ import annotations

import typing as t

import pytest
from qgis.core import QgsProject
from qgis.PyQt.QtCore import QModelIndex, QRect, Qt
from qgis.PyQt.QtWidgets import QApplication, QStyle, QTreeView, QWidget

from topology_rules.panel.dock import Dock

from .helpers import add, click_run, line_layer

DANGLES = 'line/must-not-have-dangles'
OVERLAPS = 'line/must-not-overlap-with'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
# A layer name of 40 characters, and one that cannot fit on a line at 300 px
LONG_NAME = 'Northern district road network of Cluj A'
WIDER_NAME = f'{LONG_NAME} and the county roads of Alba'
VIEWS = ['rules_tree', 'results_view']
# Widgets whose smallest width is more than this are reported when a dock is
# wider than it was told to be
NARROW = 250
# The pixels that the room of a text may be wrong by
SLACK = 10
# An int in Qt5 and a flag in Qt6
WORD_WRAP = getattr(Qt.TextFlag.TextWordWrap, 'value', Qt.TextFlag.TextWordWrap)


@pytest.fixture
def narrow(dock: Dock) -> Dock:
    """The dock at 300 px, with rules and their errors, some of them with long
    names."""
    assert len(LONG_NAME) == 40
    lakes = line_layer('LINESTRING(0 0, 10 0)', name='lakes')
    countries = line_layer('LINESTRING(5 0, 15 0)', name='countries')
    long = line_layer(*CHAIN, name=LONG_NAME)
    wider = line_layer(*CHAIN, name=WIDER_NAME)
    for layer in (lakes, countries, long, wider):
        QgsProject.instance().addMapLayer(layer)
    add(dock, OVERLAPS, lakes, countries)
    add(dock, DANGLES, long)
    add(dock, DANGLES, wider)
    add(dock, DANGLES, lakes)
    click_run(dock)
    dock.resize(300, 900)
    dock.show()
    QApplication.processEvents()
    return dock


def what_forces_the_width(dock: Dock) -> str:
    """Why a dock is wider than it was told to be: the widgets in it whose
    smallest width is large, and the font that sets their size. A dock cannot
    be narrower than the widest of what it holds."""
    lines = [
        f'{dock.width()} px wide, the smallest it can be is '
        f'{dock.minimumSizeHint().width()} px (minimum width '
        f'{dock.minimumWidth()}), font {dock.font().family()!r} at '
        f'{dock.font().pointSizeF()} pt, {dock.logicalDpiX()} dpi'
    ]
    for widget in dock.findChildren(QWidget):
        smallest = widget.minimumSizeHint().width()
        if widget.isVisibleTo(dock) and smallest > NARROW:
            text = widget.text() if hasattr(widget, 'text') else ''
            lines.append(
                f'{type(widget).__name__} {widget.objectName()!r} '
                f'{str(text)[:40]!r}: smallest width {smallest} px, '
                f'now {widget.width()} px'
            )
    return '\n'.join(lines)


def show_view(dock: Dock, name: str) -> QTreeView:
    view: QTreeView = getattr(dock, name)
    for index in range(dock.tabs.count()):
        page = dock.tabs.widget(index)
        if page is not None and (page is view or page.isAncestorOf(view)):
            dock.tabs.setCurrentIndex(index)
    view.expandAll()
    view.doItemsLayout()
    QApplication.processEvents()
    return view


def cells(view: QTreeView) -> t.Iterator[QModelIndex]:
    """Every cell that is on show."""
    model = view.model()
    assert model is not None

    def walk(parent: QModelIndex) -> t.Iterator[QModelIndex]:
        for row in range(model.rowCount(parent)):
            for column in range(model.columnCount(parent)):
                if not view.isColumnHidden(column):
                    yield model.index(row, column, parent)
            yield from walk(model.index(row, 0, parent))

    yield from walk(QModelIndex())


def text_of(index: QModelIndex) -> str:
    return str(index.data(Qt.ItemDataRole.DisplayRole) or '')


def with_text(view: QTreeView) -> list[QModelIndex]:
    return [index for index in cells(view) if text_of(index)]


def room(view: QTreeView, index: QModelIndex, slack: int = 0) -> int:
    """The width that the text of a cell has, less the check box and the icon
    and the margins of the text, and the slack."""
    style = view.style()
    width = view.visualRect(index).width()
    margin = style.pixelMetric(QStyle.PixelMetric.PM_FocusFrameHMargin) + 1
    taken = 2 * margin
    if index.data(Qt.ItemDataRole.DecorationRole) is not None:
        icon = view.iconSize().width()
        if icon <= 0:
            icon = style.pixelMetric(QStyle.PixelMetric.PM_SmallIconSize)
        taken += icon + margin
    if index.data(Qt.ItemDataRole.CheckStateRole) is not None:
        taken += (
            style.pixelMetric(QStyle.PixelMetric.PM_IndicatorWidth) + margin
        )
    return max(width - taken + slack, 1)


def wrapped(view: QTreeView, index: QModelIndex, slack: int = 0) -> QRect:
    """The text as the cell would wrap it with that room, in as much height as
    it takes."""
    return view.fontMetrics().boundingRect(
        QRect(0, 0, room(view, index, slack), 100_000),
        WORD_WRAP,
        text_of(index),
    )


def lines(view: QTreeView, index: QModelIndex, slack: int = 0) -> int:
    height = wrapped(view, index, slack).height()
    return max(round(height / view.fontMetrics().lineSpacing()), 1)


def row_of(view: QTreeView, index: QModelIndex) -> list[QModelIndex]:
    model = view.model()
    assert model is not None
    parent = index.parent()
    return [
        model.index(index.row(), column, parent)
        for column in range(model.columnCount(parent))
        if not view.isColumnHidden(column)
    ]


def most_lines(view: QTreeView, index: QModelIndex, slack: int) -> int:
    """The lines that a row needs: those of its cell with the most."""
    return max(lines(view, cell, slack) for cell in row_of(view, index))


def first_cells(view: QTreeView) -> list[QModelIndex]:
    return [index for index in cells(view) if index.column() == 0]


def one_line(view: QTreeView) -> int:
    return min(view.visualRect(index).height() for index in first_cells(view))


# The data of the tests is there


@pytest.mark.parametrize('name', VIEWS)
def test_the_lists_have_the_long_names(narrow: Dock, name: str):
    texts = [text_of(index) for index in with_text(show_view(narrow, name))]
    assert any(LONG_NAME in text for text in texts)
    assert any(WIDER_NAME in text for text in texts)
    assert any('Must not overlap with countries' in text for text in texts)


@pytest.mark.parametrize('name', VIEWS)
def test_the_dock_is_300_px_wide(narrow: Dock, name: str):
    show_view(narrow, name)
    assert narrow.width() == 300, what_forces_the_width(narrow)


# Nothing is elided


@pytest.mark.parametrize('name', VIEWS)
def test_every_item_fits_its_cell_when_wrapped(narrow: Dock, name: str):
    view = show_view(narrow, name)
    too_wide: list[str] = []
    too_high: list[str] = []
    for index in with_text(view):
        needed = wrapped(view, index, SLACK)
        if needed.width() > room(view, index, SLACK):
            too_wide.append(text_of(index))
        if needed.height() > view.visualRect(index).height():
            too_high.append(text_of(index))
    assert too_wide == []
    assert too_high == []


@pytest.mark.parametrize('name', VIEWS)
def test_every_cell_is_on_show(narrow: Dock, name: str):
    view = show_view(narrow, name)
    for index in with_text(view):
        assert view.visualRect(index).width() > 0, text_of(index)
        assert view.visualRect(index).height() > 0, text_of(index)


@pytest.mark.parametrize('name', VIEWS)
def test_no_item_has_an_ellipsis(narrow: Dock, name: str):
    view = show_view(narrow, name)
    texts = [text_of(index) for index in with_text(view)]
    assert not any('…' in text or '...' in text for text in texts)


@pytest.mark.parametrize('name', VIEWS)
def test_the_list_does_not_scroll_sideways(narrow: Dock, name: str):
    view = show_view(narrow, name)
    assert not view.horizontalScrollBar().isVisible()


@pytest.mark.parametrize('name', VIEWS)
def test_the_list_wraps_words(narrow: Dock, name: str):
    assert show_view(narrow, name).wordWrap()


# The rows grow where they must


@pytest.mark.parametrize('name', VIEWS)
def test_a_name_that_cannot_fit_wraps_onto_a_second_line(
    narrow: Dock, name: str
):
    view = show_view(narrow, name)
    wide = [
        index
        for index in with_text(view)
        if WIDER_NAME in text_of(index) and index.column() == 0
    ]
    assert wide
    for index in wide:
        assert lines(view, index, SLACK) > 1, text_of(index)
        assert view.visualRect(index).height() > one_line(view)


@pytest.mark.parametrize('name', VIEWS)
def test_the_rows_that_must_wrap_are_taller(narrow: Dock, name: str):
    view = show_view(narrow, name)
    single = one_line(view)
    for index in first_cells(view):
        if most_lines(view, index, SLACK) > 1:
            assert view.visualRect(index).height() > single, text_of(index)


@pytest.mark.parametrize('name', VIEWS)
def test_the_rows_that_fit_stay_as_high_as_one_line(narrow: Dock, name: str):
    view = show_view(narrow, name)
    single = one_line(view)
    kept = [
        index
        for index in first_cells(view)
        if most_lines(view, index, -SLACK) == 1
    ]
    assert kept, 'every row wraps, so nothing is compared'
    for index in kept:
        assert view.visualRect(index).height() == single, text_of(index)


@pytest.mark.parametrize('name', VIEWS)
def test_a_row_grows_by_the_lines_it_needs(narrow: Dock, name: str):
    view = show_view(narrow, name)
    spacing = view.fontMetrics().lineSpacing()
    single = one_line(view)
    for index in first_cells(view):
        fewest = most_lines(view, index, SLACK)
        most = most_lines(view, index, -SLACK)
        height = view.visualRect(index).height()
        assert height >= single + (fewest - 1) * spacing - 1, text_of(index)
        assert height <= single + (most - 1) * spacing + 1, text_of(index)


# A change in the width wraps again


@pytest.mark.parametrize('name', VIEWS)
def test_a_wider_dock_makes_no_row_taller(narrow: Dock, name: str):
    view = show_view(narrow, name)
    before = [view.visualRect(index).height() for index in first_cells(view)]
    narrow.resize(700, 900)
    QApplication.processEvents()
    view = show_view(narrow, name)
    after = [view.visualRect(index).height() for index in first_cells(view)]
    assert len(after) == len(before)
    assert all(a <= b for a, b in zip(after, before, strict=True))
    assert sum(after) < sum(before)


@pytest.mark.parametrize('name', VIEWS)
def test_a_narrower_dock_still_fits_every_item(narrow: Dock, name: str):
    narrow.resize(240, 900)
    QApplication.processEvents()
    view = show_view(narrow, name)
    for index in with_text(view):
        needed = wrapped(view, index, SLACK)
        height = view.visualRect(index).height()
        assert needed.height() <= height, text_of(index)
