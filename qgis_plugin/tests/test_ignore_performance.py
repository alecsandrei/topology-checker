"""Many errors and many rules: counting the errors that are left and building
the tree again take about as long as it takes to look at each error once."""

from __future__ import annotations

import pytest
from qgis.PyQt.QtCore import QModelIndex
from qgis.PyQt.QtWidgets import QApplication

from topology_rules.panel import controller, ignored
from topology_rules.panel.dock import Dock
from topology_rules.panel.entries import Entry
from topology_rules.panel.rows import Row

RULES = 40
# Errors enough to show a search of all of them for each of the rules
MANY = 20_000


def filled(dock: Dock, rows: int) -> list[Row]:
    """Rules in the list, and the results of a run with errors for each of
    them, of which every tenth is ignored."""
    rule = dock.form.rules[0]
    for number in range(RULES):
        dock.rule_list.add(Entry(rule, f'layer{number}', f'layer{number}', 'm'))
    keys = [f'e{number}' for number in range(RULES)]
    dock.run_keys = {
        entry.key: key for entry, key in zip(dock.entries, keys, strict=True)
    }
    errors = [
        Row(
            rule.name,
            'dangle',
            f'layer{number % RULES}',
            number,
            '',
            None,
            f'a{number % RULES}',
            '',
            'errors',
            number,
            keys[number % RULES],
            str(number),
        )
        for number in range(rows)
    ]
    dock.results.ignored.add(ignored.stamp(errors[::10], {}))
    dock.results.show_rows(errors, {}, labels={key: key for key in keys})
    return errors


@pytest.fixture
def counted(monkeypatch: pytest.MonkeyPatch) -> list[int]:
    """The number of times that the fingerprint of an error is worked out."""
    count = [0]
    original = ignored.fingerprint

    def counting(*args: object) -> str:
        count[0] += 1
        return original(*args)  # type: ignore[arg-type]

    monkeypatch.setattr(ignored, 'fingerprint', counting)
    return count


def test_the_fingerprint_of_an_error_is_worked_out_once_per_error(
    dock: Dock, counted: list[int]
):
    rows = 4000
    errors = filled(dock, rows)
    counted[0] = 0
    dock.results.show_rows(
        errors, {}, labels={f'e{number}': '' for number in range(RULES)}
    )
    controller.recount(dock)
    assert counted[0] <= rows


# The work is counted and not the seconds: the seconds depend on the machine,
# and a slow one fails a test that a quick one passes. A search of all the
# errors for each rule would show here as many times more looks at the errors


@pytest.fixture
def looks(monkeypatch: pytest.MonkeyPatch) -> list[int]:
    """The number of times that the list of ignored errors is asked about an
    error."""
    count = [0]
    original = ignored.Ignored.has

    def counting(self: ignored.Ignored, row: Row) -> bool:
        count[0] += 1
        return original(self, row)

    monkeypatch.setattr(ignored.Ignored, 'has', counting)
    return count


def settled(dock: Dock) -> None:
    """Lets what is waiting in the event loop happen, and leaves nothing
    selected, so that the count is of the work of the call that is measured
    and not of what a timer, a layout or a selection adds when the machine is
    slow. The errors of a node that is selected are looked at again."""
    for _ in range(5):
        QApplication.processEvents()
    dock.results_view.clearSelection()
    dock.results_view.setCurrentIndex(QModelIndex())
    for _ in range(5):
        QApplication.processEvents()


def test_counting_the_errors_that_are_left_looks_at_each_error_once(
    dock: Dock, counted: list[int], looks: list[int]
):
    filled(dock, MANY)
    settled(dock)
    counted[0] = looks[0] = 0
    controller.recount(dock)
    assert looks[0] <= MANY
    # The fingerprints are kept, and not worked out again
    assert counted[0] == 0


def test_building_the_tree_again_looks_at_each_error_a_few_times(
    dock: Dock, counted: list[int], looks: list[int]
):
    filled(dock, MANY)
    settled(dock)
    counted[0] = looks[0] = 0
    dock.results.refresh()
    # To count the ones that are ignored, to tell which to show, to draw them
    # and to filter the map: a few times, and not once for each of the rules
    assert looks[0] <= 5 * MANY
    assert counted[0] <= MANY
