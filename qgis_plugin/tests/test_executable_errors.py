"""When the tool that is set cannot be used, the setup page says which one it
is: its path, so that the user knows what to change, and offers to choose
another."""

from __future__ import annotations

import collections.abc as c
from pathlib import Path

import pytest
from qgis.gui import QgisInterface

from topology_rules.core import binary
from topology_rules.panel.dock import Dock

from .helpers import probe_executable, script_executable, wait_for

# Rules as an older tool lists them, without the geometry of the layers
OLD_RULES = [
    {'name': 'line/must-not-have-dangles', 'description': 'Old rule.'},
]
KINDS = ['old rules', 'no rules', 'fails', 'too slow']


@pytest.fixture
def unusable(
    qgis_iface: QgisInterface,
    configure: c.Callable[[str | Path], None],
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> c.Callable[[str], tuple[Dock, Path]]:
    """Makes a dock for a tool that cannot be used, of a kind."""
    docks: list[Dock] = []

    def make(kind: str) -> tuple[Dock, Path]:
        if kind == 'too slow':
            monkeypatch.setattr(binary, 'PROBE_TIMEOUT', 0.5)
        tool = probe_executable(
            tmp_path,
            rules=OLD_RULES if kind == 'old rules' else [],
            broken=kind == 'fails',
            delay=5.0 if kind == 'too slow' else 0.0,
        )
        configure(tool)
        dock = Dock(qgis_iface)
        docks.append(dock)
        dock.refresh()
        wait_for(lambda: dock.probing is None, 30)
        return dock, tool

    yield make
    for dock in docks:
        dock.deleteLater()


@pytest.mark.parametrize('kind', KINDS)
def test_the_setup_page_names_the_tool_that_cannot_be_used(
    unusable: c.Callable[[str], tuple[Dock, Path]], kind: str
):
    dock, tool = unusable(kind)
    assert not dock.available
    assert str(tool) in dock.status.text()


@pytest.mark.parametrize('kind', KINDS)
def test_the_setup_page_offers_to_choose_another_tool(
    unusable: c.Callable[[str], tuple[Dock, Path]], kind: str
):
    dock, _ = unusable(kind)
    assert not dock.setup_page.isHidden()
    assert not dock.browse_button.isHidden()


# The version of the tool that the plugin needs


def test_a_tool_that_is_too_old_says_which_version_is_needed(
    unusable: c.Callable[[str], tuple[Dock, Path]],
):
    dock, _ = unusable('old rules')
    assert '0.2.0' in dock.status.text()


def test_a_tool_that_reports_0_1_0_but_lists_layer_kinds_is_accepted(
    qgis_iface: QgisInterface,
    configure: c.Callable[[str | Path], None],
    tmp_path: Path,
):
    # A build that is numbered 0.1.0 is taken by what it can do: its rules
    # say which kinds of layer they are about
    tool = script_executable(
        tmp_path,
        "if sys.argv[1:] in (['--version'], ['-V'], ['--json', '--version']):\n"
        "    print('topology-checker 0.1.0')\n"
        '    sys.exit(0)\n'
        'sys.exit(2)\n',
    )
    configure(tool)
    dock = Dock(qgis_iface)
    dock.refresh()
    wait_for(lambda: dock.available, 30)
    assert dock.setup_page.isHidden()
    dock.deleteLater()
