"""The fake tools of the tests on Windows, which ignores a shebang: a script
and a `.cmd` beside it that starts the script."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from .helpers import hand_over, write_tool

SOURCE = '#!/usr/bin/env python3\nprint(1)\n'


def test_a_fake_tool_is_the_script_itself_where_a_shebang_runs_it(
    tmp_path: Path,
):
    if sys.platform == 'win32':
        pytest.skip('Windows has no shebang')
    path = write_tool(tmp_path / 'tool', SOURCE)
    assert path == tmp_path / 'tool'
    assert path.read_text() == SOURCE
    assert path.stat().st_mode & 0o100


def test_a_fake_tool_on_windows_is_a_launcher_of_a_script_beside_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(sys, 'platform', 'win32')
    path = write_tool(tmp_path / 'tool', SOURCE)
    assert path == tmp_path / 'tool.cmd'
    assert (tmp_path / 'tool.py').read_text() == SOURCE
    launcher = path.read_text()
    assert f'"{sys.executable}"' in launcher
    # The script of the launcher is the one beside it, whatever the folder
    assert '"%~dp0tool.py"' in launcher
    assert launcher.rstrip().endswith('%*')


def test_a_fake_tool_on_windows_leaves_no_file_with_the_name_of_the_tool(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(sys, 'platform', 'win32')
    write_tool(tmp_path / 'tool', SOURCE)
    assert not (tmp_path / 'tool').exists()


def test_a_fake_tool_hands_over_to_the_real_tool_by_exec_where_that_exists():
    assert hand_over('/real').startswith("os.execv('/real'")


def test_a_fake_tool_on_windows_waits_for_the_real_tool_and_ends_as_it_does(
    monkeypatch: pytest.MonkeyPatch,
):
    monkeypatch.setattr(sys, 'platform', 'win32')
    lines = hand_over('C:/real.exe')
    assert 'subprocess.call' in lines
    assert 'sys.exit(' in lines
    assert 'execv' not in lines
