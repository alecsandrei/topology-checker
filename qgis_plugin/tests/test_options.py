"""The options of a run, tucked away in a box that opens when they are
wanted: the tolerance and what part of the layers to check."""

from __future__ import annotations

import collections.abc as c

import pytest
from qgis.gui import QgisInterface, QgsCollapsibleGroupBox

from topology_rules.panel.dock import Dock


def options_box(dock: Dock) -> QgsCollapsibleGroupBox:
    boxes = [
        box
        for box in dock.findChildren(QgsCollapsibleGroupBox)
        if box.title().startswith('Options')
    ]
    assert len(boxes) == 1, [box.title() for box in boxes]
    return boxes[0]


@pytest.fixture
def again(qgis_iface: QgisInterface) -> c.Iterator[c.Callable[[], Dock]]:
    """Makes another dock, as when the plugin is loaded again."""
    docks: list[Dock] = []

    def make() -> Dock:
        dock = Dock(qgis_iface)
        dock.refresh()
        docks.append(dock)
        return dock

    yield make
    for dock in docks:
        dock.deleteLater()


# The box


def test_the_options_are_in_a_collapsible_box(dock: Dock):
    box = options_box(dock)
    assert box.title().startswith('Options')


@pytest.mark.parametrize('name', ['tolerance', 'selected_only', 'extent_only'])
def test_the_box_holds_an_option(dock: Dock, name: str):
    assert options_box(dock).isAncestorOf(getattr(dock, name))


def test_the_box_is_collapsed_at_first(dock: Dock):
    assert options_box(dock).isCollapsed()


def test_the_run_button_is_outside_the_box(dock: Dock):
    assert not options_box(dock).isAncestorOf(dock.run_button)


# The title says what is set


def test_the_title_is_plain_when_nothing_is_set(dock: Dock):
    title = options_box(dock).title().lower()
    for word in ('tolerance', 'selected', 'extent'):
        assert word not in title


def test_the_title_mentions_a_tolerance(dock: Dock):
    dock.tolerance.setValue(0.5)
    assert 'tolerance' in options_box(dock).title().lower()


def test_the_title_mentions_selected_features(dock: Dock):
    dock.selected_only.setChecked(True)
    assert 'selected' in options_box(dock).title().lower()


def test_the_title_mentions_the_visible_extent(dock: Dock):
    dock.extent_only.setChecked(True)
    assert 'extent' in options_box(dock).title().lower()


def test_the_title_mentions_everything_that_is_set(dock: Dock):
    dock.tolerance.setValue(0.5)
    dock.selected_only.setChecked(True)
    dock.extent_only.setChecked(True)
    title = options_box(dock).title().lower()
    for word in ('tolerance', 'selected', 'extent'):
        assert word in title


def test_the_title_goes_back_when_the_options_are_cleared(dock: Dock):
    dock.tolerance.setValue(0.5)
    dock.selected_only.setChecked(True)
    dock.tolerance.clear()
    dock.selected_only.setChecked(False)
    title = options_box(dock).title().lower()
    for word in ('tolerance', 'selected', 'extent'):
        assert word not in title


def test_the_title_still_starts_with_options(dock: Dock):
    dock.tolerance.setValue(0.5)
    dock.extent_only.setChecked(True)
    assert options_box(dock).title().startswith('Options')


# Remembered


def test_an_open_box_stays_open_in_the_next_dock(
    dock: Dock, again: c.Callable[[], Dock]
):
    options_box(dock).setCollapsed(False)
    assert not options_box(again()).isCollapsed()


def test_a_closed_box_stays_closed_in_the_next_dock(
    dock: Dock, again: c.Callable[[], Dock]
):
    box = options_box(dock)
    box.setCollapsed(False)
    box.setCollapsed(True)
    assert options_box(again()).isCollapsed()
