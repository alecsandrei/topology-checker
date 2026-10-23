"""A kind of error with thousands of errors: it opens at once, as the view
measures each row once at most, and every error is in reach with End and the
scroll bar. What it does with its errors covers all of them.

The work is counted and not the seconds, as in the tests of ignoring. Some
styles scroll by pixels, and their views measure every row they show after
each layout, so the tests run in both ways."""

from __future__ import annotations

import collections.abc as c
import time

import pytest
from qgis.core import QgsFeature, QgsGeometry, QgsProject, QgsVectorLayer
from qgis.PyQt.QtCore import QModelIndex, Qt
from qgis.PyQt.QtTest import QTest
from qgis.PyQt.QtWidgets import QAbstractItemView, QApplication, QTreeView

from topology_rules.panel import wrapping
from topology_rules.panel.dock import Dock
from topology_rules.panel.ignored import stamp
from topology_rules.panel.rows import Row
from topology_rules.panel.tree import ErrorModel

from .helpers import add, click_run, queue_column, shown

MANY = 5_000
# A text that wraps in the dock at 300 px, and not at 900 px
LONG = 'roads of the county ' * 3
DANGLES = 'line/must-not-have-dangles'
SCROLLING = {
    'by item': QAbstractItemView.ScrollMode.ScrollPerItem,
    'by pixel': QAbstractItemView.ScrollMode.ScrollPerPixel,
}


def errors(
    count: int, key: str = 'e0', layer: str = 'roads', numbered: bool = True
) -> list[Row]:
    """The dangles of a run, of a rule of the key, on the features of the
    layer or on none."""
    return [
        Row(
            DANGLES,
            'dangle',
            layer,
            number if numbered else None,
            '',
            None,
            'a',
            '',
            'errors',
            number,
            key,
            str(number),
        )
        for number in range(count)
    ]


def settle() -> None:
    for _ in range(5):
        QApplication.processEvents()


def model_of(dock: Dock) -> ErrorModel:
    return dock.results.model


def kind_of(dock: Dock, group: int = 0) -> QModelIndex:
    model = model_of(dock)
    return model.index(0, 0, model.index(group, 0))


def error_of(dock: Dock, row: int, group: int = 0) -> QModelIndex:
    kind = kind_of(dock, group)
    return model_of(dock).index(row % model_of(dock).rowCount(kind), 0, kind)


def opened(
    dock: Dock,
    rows: list[Row],
    groups: int = 1,
    shut: int = 0,
    title: str = 'roads',
) -> QTreeView:
    """The view of the errors in sight, with the kind of each group open but
    the last few."""
    dock.resize(300, 600)
    dock.show()
    dock.tabs.setCurrentIndex(1)
    labels = {f'e{number}': f'{title} {number}' for number in range(groups)}
    dock.results.show_rows(rows, {}, labels=labels)
    settle()
    for group in range(groups - shut):
        kind = kind_of(dock, group)
        dock.results_view.expand(kind.parent())
        dock.results_view.expand(kind)
    settle()
    return dock.results_view


def in_sight(view: QTreeView, index: QModelIndex) -> bool:
    viewport = view.viewport()
    assert viewport is not None
    rect = view.visualRect(index)
    return rect.isValid() and viewport.rect().intersects(rect)


def height(view: QTreeView, index: QModelIndex) -> int:
    return view.visualRect(index).height()


def press(view: QTreeView, key: Qt.Key) -> None:
    QTest.keyClick(view, key)
    settle()


@pytest.fixture(params=list(SCROLLING))
def scrolling(request: pytest.FixtureRequest, dock: Dock) -> str:
    dock.results_view.setVerticalScrollMode(SCROLLING[request.param])
    return request.param


@pytest.fixture
def measured(monkeypatch: pytest.MonkeyPatch) -> list[int]:
    """The number of rows that the view measures."""
    count = [0]
    original = wrapping.WrapDelegate.sizeHint

    def counting(self: wrapping.WrapDelegate, *args: object) -> object:
        count[0] += 1
        return original(self, *args)

    monkeypatch.setattr(wrapping.WrapDelegate, 'sizeHint', counting)
    return count


def test_a_kind_has_an_item_for_each_error(dock: Dock):
    opened(dock, errors(MANY))
    assert model_of(dock).rowCount(kind_of(dock)) == MANY
    assert kind_of(dock).data() == f'Dangles ({MANY})'
    assert model_of(dock).index(0, 0).data() == f'roads 0 ({MANY})'


def test_the_errors_follow_in_their_order(dock: Dock):
    opened(dock, errors(MANY))
    texts = [error_of(dock, row).data() for row in range(MANY)]
    assert texts == [f'roads {number}' for number in range(MANY)]


def test_errors_without_features_are_numbered(dock: Dock):
    opened(dock, errors(MANY, numbered=False))
    assert error_of(dock, 0).data() == 'Dangle 1'
    assert error_of(dock, -1).data() == f'Dangle {MANY}'


def test_opening_a_kind_measures_each_row_once_at_most(
    dock: Dock, scrolling: str, measured: list[int]
):
    # A view that scrolls by items measures the rows in sight, however many
    # there are
    opened(dock, errors(MANY))
    assert 0 < measured[0] <= (MANY + 50 if scrolling == 'by pixel' else 500)


def test_end_goes_to_the_last_error(dock: Dock, scrolling: str):
    view = opened(dock, errors(MANY))
    view.setCurrentIndex(error_of(dock, 0))
    press(view, Qt.Key.Key_End)
    assert view.currentIndex() == error_of(dock, -1)
    assert in_sight(view, error_of(dock, -1))


def test_the_scroll_bar_reaches_the_last_error(dock: Dock, scrolling: str):
    view = opened(dock, errors(MANY))
    bar = view.verticalScrollBar()
    assert bar is not None
    bar.setValue(bar.maximum())
    settle()
    assert in_sight(view, error_of(dock, -1))
    # The bar spans all of the errors: a new layout keeps its range
    most = bar.maximum()
    view.doItemsLayout()
    settle()
    assert bar.maximum() == most


@pytest.mark.parametrize('how', ['star', 'recursively'])
def test_opening_a_group_with_all_in_it_lays_out_its_errors(
    dock: Dock, scrolling: str, how: str
):
    view = opened(dock, errors(500) + errors(500, 'e1'), groups=2, shut=1)
    # The first group open with its kinds, the second closed
    second = model_of(dock).index(1, 0)
    if how == 'star':
        view.setCurrentIndex(second)
        press(view, Qt.Key.Key_Asterisk)
    else:
        view.expandRecursively(second)
        settle()
    bar = view.verticalScrollBar()
    assert bar is not None
    assert view.isExpanded(kind_of(dock, 1))
    assert view.visualRect(error_of(dock, 0, group=1)).isValid()
    most = bar.maximum()
    view.doItemsLayout()
    settle()
    assert bar.maximum() == most
    view.setCurrentIndex(second)
    press(view, Qt.Key.Key_End)
    assert view.currentIndex() == error_of(dock, -1, group=1)


def test_resizing_measures_only_the_rows_that_wrap(
    dock: Dock, scrolling: str, measured: list[int]
):
    opened(dock, errors(MANY))
    measured[0] = 0
    for width in (280, 340, 260, 400, 300):
        dock.resize(width, 600)
        settle()
    assert measured[0] < 100


def test_a_title_that_wraps_is_measured_again(dock: Dock, scrolling: str):
    # The title of the group wraps in the narrow dock, and its errors do not
    view = opened(dock, errors(MANY), title=LONG)
    group = model_of(dock).index(0, 0)
    narrow = height(view, group)
    error = height(view, error_of(dock, 0))
    dock.resize(900, 600)
    settle()
    assert height(view, group) < narrow
    assert height(view, error_of(dock, 0)) == error
    dock.resize(300, 600)
    settle()
    assert height(view, group) == narrow


def test_errors_wrap_when_the_column_gets_narrower_than_them(
    dock: Dock, scrolling: str
):
    view = opened(dock, errors(MANY, layer=LONG))
    dock.resize(900, 600)
    settle()
    one_line = height(view, kind_of(dock))
    assert height(view, error_of(dock, 0)) == one_line
    assert height(view, error_of(dock, -1)) == one_line
    dock.resize(300, 600)
    settle()
    assert height(view, error_of(dock, 0)) > one_line
    assert height(view, error_of(dock, -1)) > one_line


def test_errors_that_wrap_are_measured_a_few_times_when_the_column_changes(
    dock: Dock, scrolling: str, measured: list[int]
):
    # A size hint changed for each row was a layout of all the rows for
    # each of them
    count = 500
    opened(dock, errors(count, layer=LONG))
    measured[0] = 0
    dock.resize(400, 600)
    settle()
    assert 0 < measured[0] <= 4 * count


@pytest.mark.parametrize('node', ['group', 'kind'])
def test_ignoring_covers_all_of_the_errors(dock: Dock, node: str):
    opened(dock, errors(MANY))
    kind = kind_of(dock)
    dock.results_view.setCurrentIndex(kind if node == 'kind' else kind.parent())
    dock.results.ignore_action.trigger()
    assert dock.results.active() == []
    assert dock.show_ignored.text() == f'Show ignored ({MANY})'
    assert model_of(dock).rowCount() == 0


def test_the_ignored_errors_show_with_all_of_their_count(dock: Dock):
    opened(dock, errors(MANY))
    dock.results_view.setCurrentIndex(kind_of(dock))
    dock.results.ignore_action.trigger()
    dock.show_ignored.setChecked(True)
    assert kind_of(dock).data() == f'Dangles ({MANY})'
    assert model_of(dock).index(0, 0).data() == f'roads 0 ({MANY})'
    assert model_of(dock).rowCount(kind_of(dock)) == MANY


def test_stopping_to_ignore_a_kind_covers_all_of_the_errors(dock: Dock):
    opened(dock, errors(MANY))
    dock.results_view.setCurrentIndex(kind_of(dock))
    dock.results.ignore_action.trigger()
    dock.show_ignored.setChecked(True)
    dock.results_view.setCurrentIndex(kind_of(dock))
    assert dock.results.restore_action.isEnabled()
    dock.results.restore_action.trigger()
    assert len(dock.results.active()) == MANY


def test_the_errors_that_are_ignored_are_grey(dock: Dock):
    rows = errors(MANY)
    # Every other error
    dock.results.ignored.add(stamp(rows[::2], {}))
    dock.show_ignored.setChecked(True)
    opened(dock, rows)
    grey = [
        error_of(dock, row).data(Qt.ItemDataRole.ForegroundRole) is not None
        for row in (0, 1, MANY - 2, MANY - 1)
    ]
    assert grey == [True, False, True, False]


# A layer through the dock, with tens of thousands of dangles


SPIKES = 15_000
# Both ends of each separate line
MANY_DANGLES = 2 * SPIKES
# The longest that a step may keep the interface busy. Each step took
# minutes when the view measured every row once for each row
MAX_STALL = 10.0


def spiky_layer() -> QgsVectorLayer:
    """Lines far from each other, each with two free ends."""
    layer = QgsVectorLayer('LineString?crs=EPSG:32633', 'spikes', 'memory')
    features: list[QgsFeature] = []
    for index in range(SPIKES):
        feature = QgsFeature()
        feature.setGeometry(
            QgsGeometry.fromWkt(f'LINESTRING({3 * index} 0, {3 * index + 1} 0)')
        )
        features.append(feature)
    assert layer.dataProvider().addFeatures(features)[0]
    layer.updateExtents()
    QgsProject.instance().addMapLayer(layer)
    return layer


def longest_step(steps: c.Iterable[c.Callable[[], object]]) -> float:
    """The longest that a step and the events after it kept the interface
    busy."""
    longest = 0.0
    for step in steps:
        before = time.monotonic()
        step()
        settle()
        longest = max(longest, time.monotonic() - before)
    return longest


@pytest.mark.slow
def test_a_kind_of_many_dangles_opens_without_freezing(
    dock: Dock, scrolling: str, measured: list[int]
):
    add(dock, DANGLES, spiky_layer())
    click_run(dock)
    assert queue_column(dock, 3) == [str(MANY_DANGLES)]
    dock.resize(300, 600)
    dock.show()
    dock.tabs.setCurrentIndex(1)
    settle()
    view = dock.results_view
    kind = kind_of(dock)
    assert kind.data() == f'Dangles ({MANY_DANGLES})'
    bar = view.verticalScrollBar()
    assert bar is not None
    measured[0] = 0
    opening = longest_step(
        [lambda: view.expand(kind.parent()), lambda: view.expand(kind)]
    )
    assert measured[0] <= MANY_DANGLES + 50
    stall = longest_step(
        [
            lambda: view.setCurrentIndex(error_of(dock, 0)),
            lambda: QTest.keyClick(view, Qt.Key.Key_End),
            lambda: bar.setValue(0),
            lambda: bar.setValue(bar.maximum()),
            lambda: dock.resize(280, 600),
        ]
    )
    assert view.currentIndex() == error_of(dock, -1)
    assert in_sight(view, error_of(dock, -1))
    assert max(opening, stall) < MAX_STALL
    assert shown(dock) == MANY_DANGLES
