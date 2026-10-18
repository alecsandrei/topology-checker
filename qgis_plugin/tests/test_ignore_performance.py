"""Many errors and many rules: counting the errors that are left and building
the tree again take about as long as it takes to look at each error once."""

from __future__ import annotations

import time

import pytest

from topology_rules.panel import controller, ignored
from topology_rules.panel.dock import Dock
from topology_rules.panel.entries import Entry
from topology_rules.panel.rows import Row

RULES = 40
# Seconds that counting and building the tree again may take, for many rows
BUDGET = 2.0


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


@pytest.mark.slow
def test_counting_the_errors_that_are_left_is_quick_for_many_errors(dock: Dock):
    filled(dock, 100_000)
    started = time.perf_counter()
    controller.recount(dock)
    assert time.perf_counter() - started < BUDGET


@pytest.mark.slow
def test_building_the_tree_again_is_quick_for_many_errors(dock: Dock):
    filled(dock, 100_000)
    started = time.perf_counter()
    dock.results.refresh()
    assert time.perf_counter() - started < BUDGET
