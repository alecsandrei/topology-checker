"""Asking tools for their rules on other threads: answers that arrive late, and
answers that arrive together."""

from __future__ import annotations

import collections.abc as c
import sys
import threading
from pathlib import Path

import pytest
from qgis.core import QgsApplication
from qgis.gui import QgisInterface

from topology_rules.core import binary
from topology_rules.panel.dock import Dock
from topology_rules.plugin import TopologyRules

from .helpers import (
    RULES,
    calls,
    counting_executable,
    probe_executable,
    wait_for,
)

FIRST, SECOND = (rule['name'] for rule in RULES)
# What a thread does with the cache of answers, and how often
THREADS = 4
CALLS = 300
# Seconds that the threads may take together
JOIN_SECONDS = 60


def known_rules(dock: Dock) -> list[str]:
    """The names of the rules that the dock got from the tool."""
    return [rule.name for rule in dock.form.rules]


@pytest.fixture
def slow_then_fast(
    dock: Dock, configure: c.Callable[[str | Path], None], tmp_path: Path
) -> c.Callable[[bool], Path]:
    """Starts asking a slow tool for its rules, then points the dock to a fast
    one with other rules, and waits for the fast one to answer. Returns the
    gate that lets the slow one answer; `broken` makes it fail."""
    # The dock lets go of the task of the slow tool when it asks the fast one.
    # Its answer is waited for here, whether or not the dock still has it
    tasks: list[object] = []

    def start(broken: bool) -> Path:
        wait_for(lambda: dock.probing is None)
        gate = tmp_path / 'gate'
        slow, fast = tmp_path / 'slow', tmp_path / 'fast'
        slow.mkdir()
        fast.mkdir()
        configure(
            probe_executable(slow, rules=[RULES[0]], gate=gate, broken=broken)
        )
        dock.refresh()
        tasks.append(dock.probe_task)
        configure(probe_executable(fast, rules=[RULES[1]]))
        dock.refresh()
        wait_for(lambda: known_rules(dock) == [SECOND])
        return gate

    return start


def let_it_answer(gate: Path) -> None:
    gate.touch()
    wait_for(lambda: not QgsApplication.taskManager().activeTasks())
    for _ in range(20):
        QgsApplication.processEvents()


def test_a_late_answer_of_the_tool_that_was_left_does_not_change_the_rules(
    dock: Dock, slow_then_fast: c.Callable[[bool], Path]
):
    let_it_answer(slow_then_fast(False))
    assert known_rules(dock) == [SECOND]


def test_a_late_failure_of_the_tool_that_was_left_does_not_hide_the_dock(
    dock: Dock, slow_then_fast: c.Callable[[bool], Path]
):
    let_it_answer(slow_then_fast(True))
    assert dock.available
    assert dock.setup_page.isHidden()
    assert known_rules(dock) == [SECOND]


def test_the_tool_that_was_left_does_not_stay_marked_as_being_asked(
    dock: Dock, slow_then_fast: c.Callable[[bool], Path]
):
    let_it_answer(slow_then_fast(False))
    assert dock.probing is None


# The cache of answers is used by every thread that asks


def test_threads_that_ask_together_do_not_fail_when_old_answers_are_dropped(
    monkeypatch: pytest.MonkeyPatch,
):
    monkeypatch.setattr(binary.Probe, 'ask', staticmethod(lambda path: ()))
    probe = binary.Probe()
    errors: list[BaseException] = []
    start = threading.Barrier(THREADS, timeout=JOIN_SECONDS)

    def work(number: int) -> None:
        try:
            start.wait()
            for call in range(CALLS):
                probe(f'tool-{number}-{call}', call)
        except BaseException as error:
            errors.append(error)

    threads = [
        threading.Thread(target=work, args=(number,), daemon=True)
        for number in range(THREADS)
    ]
    interval = sys.getswitchinterval()
    # Threads take turns often, as they do when each of them also waits for a
    # program to answer
    sys.setswitchinterval(1e-5)
    try:
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(JOIN_SECONDS)
    finally:
        sys.setswitchinterval(interval)
    # A thread that is still going is a failure and not a hang of the run, as
    # it can be where threads are emulated, such as a container for another
    # processor
    assert not [thread for thread in threads if thread.is_alive()]
    assert errors == []


# The tool is started once when the plugin starts


def test_the_tool_is_asked_for_its_rules_once_when_the_plugin_starts(
    qgis_iface: QgisInterface,
    qgis_processing: None,
    executable: str,
    configure: c.Callable[[str | Path], None],
    tmp_path: Path,
):
    log = tmp_path / 'calls'
    configure(counting_executable(tmp_path, log))
    plugin = TopologyRules(qgis_iface)
    plugin.initGui()
    dock = plugin.dock
    assert dock is not None
    # What the dock does when it is shown
    dock.refresh()
    wait_for(lambda: dock.available)
    wait_for(lambda: not QgsApplication.taskManager().activeTasks(), 30)
    for _ in range(50):
        QgsApplication.processEvents()
    plugin.unload()
    assert len(calls(log, 'rules')) == 1


# The dock does not wait for the answer of a tool that it has left, however
# it is left: for a tool whose rules are known, or for one that is gone

TASKS: list[object] = []


@pytest.fixture
def asking_slow_tool(
    dock: Dock, configure: c.Callable[[str | Path], None], tmp_path: Path
) -> tuple[Path, Path]:
    """The dock is asking a slow tool for its rules. Returns the gate that
    lets it answer, and a fast tool whose rules are known."""
    wait_for(lambda: dock.probing is None)
    gate = tmp_path / 'gate'
    slow, fast = tmp_path / 'slow', tmp_path / 'fast'
    slow.mkdir()
    fast.mkdir()
    fast_tool = probe_executable(fast, rules=[RULES[1]])
    configure(fast_tool)
    dock.refresh()
    wait_for(lambda: known_rules(dock) == [SECOND])
    configure(probe_executable(slow, rules=[RULES[0]], gate=gate))
    dock.refresh()
    assert dock.probing is not None
    # Kept here, as the dock lets go of it
    TASKS.append(dock.probe_task)
    return gate, fast_tool


def test_a_late_answer_does_not_replace_rules_that_were_known_already(
    dock: Dock,
    asking_slow_tool: tuple[Path, Path],
    configure: c.Callable[[str | Path], None],
):
    gate, fast_tool = asking_slow_tool
    configure(fast_tool)
    dock.refresh()
    assert known_rules(dock) == [SECOND]
    let_it_answer(gate)
    assert known_rules(dock) == [SECOND]


def test_a_late_answer_does_not_revive_a_tool_that_is_gone(
    dock: Dock,
    asking_slow_tool: tuple[Path, Path],
    configure: c.Callable[[str | Path], None],
    tmp_path: Path,
):
    gate, _ = asking_slow_tool
    configure(tmp_path / 'gone')
    dock.refresh()
    assert not dock.available
    let_it_answer(gate)
    assert not dock.available
    assert known_rules(dock) == []


def test_the_dock_is_not_marked_as_asking_after_rules_from_the_cache(
    dock: Dock,
    asking_slow_tool: tuple[Path, Path],
    configure: c.Callable[[str | Path], None],
):
    gate, fast_tool = asking_slow_tool
    configure(fast_tool)
    dock.refresh()
    assert dock.probing is None
    let_it_answer(gate)
