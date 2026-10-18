"""Loading the algorithms asks the tool for its rules. A tool that is slow to
answer, as one is when it starts for the first time, must not freeze the main
window of QGIS while it does."""

from __future__ import annotations

import collections.abc as c
import time
from pathlib import Path

import pytest
from qgis.core import QgsApplication

from topology_rules.provider import Provider

from .helpers import RULES, probe_executable, wait_for

# Seconds that the slow tool takes to list its rules
DELAY = 1.5
# Seconds that the main thread may be held up by loading
LIMIT = 0.2


@pytest.fixture
def slow_tool(
    configure: c.Callable[[str | Path], None], tmp_path: Path
) -> Path:
    path = probe_executable(tmp_path, delay=DELAY)
    configure(path)
    return path


@pytest.fixture
def registered(qgis_processing: None) -> c.Iterator[list[Provider]]:
    """Providers that a test added, which are removed when it ends and the
    tasks that they started are over."""
    providers: list[Provider] = []
    yield providers
    wait_for(lambda: not QgsApplication.taskManager().activeTasks(), 30)
    registry = QgsApplication.processingRegistry()
    for provider in providers:
        registry.removeProvider(provider)


def add_provider(registered: list[Provider]) -> Provider:
    provider = Provider()
    registered.append(provider)
    QgsApplication.processingRegistry().addProvider(provider)
    return provider


def test_adding_the_provider_does_not_wait_for_a_slow_tool(
    slow_tool: Path, registered: list[Provider]
):
    started = time.perf_counter()
    add_provider(registered)
    assert time.perf_counter() - started < LIMIT


def test_refreshing_the_algorithms_does_not_wait_for_a_slow_tool(
    executable: str,
    configure: c.Callable[[str | Path], None],
    registered: list[Provider],
    tmp_path: Path,
):
    provider = add_provider(registered)
    wait_for(lambda: len(provider.algorithms()) > 0)
    configure(probe_executable(tmp_path, delay=DELAY))
    started = time.perf_counter()
    # What the Options dialog does on OK
    provider.refreshAlgorithms()
    assert time.perf_counter() - started < LIMIT


def test_the_algorithms_come_when_the_slow_tool_has_answered(
    slow_tool: Path, registered: list[Provider]
):
    provider = add_provider(registered)
    wait_for(lambda: len(provider.algorithms()) > len(RULES), 30)


def test_a_tool_that_was_asked_before_is_loaded_at_once(
    executable: str, registered: list[Provider]
):
    provider = add_provider(registered)
    wait_for(lambda: len(provider.algorithms()) > 0)
    started = time.perf_counter()
    provider.refreshAlgorithms()
    assert time.perf_counter() - started < LIMIT
    assert len(provider.algorithms()) > len(RULES)
