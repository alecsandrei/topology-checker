"""A run with many errors does not keep the whole report of the tool in
memory: each line of the report is read as it comes and let go, and only
what the tool says otherwise is kept."""

from __future__ import annotations

import collections.abc as c
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsProject

from topology_rules.core import checks
from topology_rules.core.runner import Completed
from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    calls,
    click_run,
    hand_over,
    line_layer,
    measuring_executable,
    output_sizes,
    queue_column,
    real_executable,
    wait_for,
    write_tool,
)

DANGLES = 'line/must-not-have-dangles'
LINES = 500
# Bytes that what is kept of the output may come to, for a run of a thousand
# errors
BUDGET = 20_000


@pytest.fixture
def measured(
    dock: Dock, tmp_path: Path, configure: c.Callable[[str | Path], None]
) -> Path:
    """A dock whose tool writes to a log how much each call printed."""
    wait_for(lambda: dock.probing is None)
    log = tmp_path / 'sizes'
    configure(measuring_executable(tmp_path, log))
    dock.refresh()
    wait_for(lambda: dock.probing is None and dock.available)
    return log


def many_errors() -> None:
    """Lines that touch nothing: each has two ends, and each end is an
    error."""
    layer = line_layer(
        *(f'LINESTRING({10 * n} 0, {10 * n + 5} 0)' for n in range(LINES)),
        name='scattered',
    )
    QgsProject.instance().addMapLayer(layer)


def test_what_is_kept_of_the_report_of_many_errors_is_small(
    dock: Dock, measured: Path, monkeypatch: pytest.MonkeyPatch
):
    kept: list[Completed] = []
    real = checks.run_validate

    def run_validate(*args: t.Any, **kwargs: t.Any) -> Completed:
        completed = real(*args, **kwargs)
        kept.append(completed)
        return completed

    monkeypatch.setattr(checks, 'run_validate', run_validate)
    many_errors()
    add(dock, DANGLES, QgsProject.instance().mapLayersByName('scattered')[0])
    click_run(dock)
    assert queue_column(dock, 3) == [str(2 * LINES)]
    # The tool said much more
    assert max(output_sizes(measured, 'validate')) > BUDGET
    assert len(kept) == 1
    assert len(kept[0].stdout) + len(kept[0].stderr) < BUDGET


# A tool that does not know the flag for a report of each rule


@pytest.fixture
def older_tool(
    dock: Dock, tmp_path: Path, configure: c.Callable[[str | Path], None]
) -> Path:
    """The real tool behind a script that logs each call and refuses the
    flag for a report of each rule, as a tool that is older than the flag
    does."""
    real = real_executable()
    assert real is not None
    wait_for(lambda: dock.probing is None)
    log = tmp_path / 'calls'
    path = tmp_path / 'topology-checker'
    tool = write_tool(
        path,
        '#!/usr/bin/env python3\n'
        'import os, sys\n'
        f'with open({str(log)!r}, "a") as log:\n'
        '    log.write(" ".join(sys.argv[1:]) + "\\n")\n'
        'if "--stream" in sys.argv:\n'
        '    print("error: unexpected argument \'--stream\' found",'
        ' file=sys.stderr)\n'
        '    sys.exit(2)\n' + hand_over(real),
    )
    configure(tool)
    dock.refresh()
    wait_for(lambda: dock.probing is None and dock.available)
    return log


def test_a_tool_without_the_flag_still_counts_the_errors(
    dock: Dock, older_tool: Path
):
    many_errors()
    add(dock, DANGLES, QgsProject.instance().mapLayersByName('scattered')[0])
    click_run(dock)
    assert queue_column(dock, 3) == [str(2 * LINES)]


def test_a_tool_that_refused_the_flag_is_not_asked_with_it_again(
    dock: Dock, older_tool: Path
):
    many_errors()
    layer = QgsProject.instance().mapLayersByName('scattered')[0]
    add(dock, DANGLES, layer)
    click_run(dock)
    click_run(dock)
    runs = calls(older_tool, 'validate')
    # The first run asks with the flag and again without it, the second
    # run only without
    assert [run for run in runs if '--stream' in run.split()] == runs[:1]
    assert len(runs) == 3
