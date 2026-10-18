"""Finding the executable and reading the rules that it lists."""

from __future__ import annotations

import collections.abc as c
from pathlib import Path

import pytest

from topology_rules.core import binary
from topology_rules.enums import Group
from topology_rules.exceptions import ExecutableError

from .helpers import RULES, fake_executable


def test_configured_path_wins_over_path(
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    monkeypatch: pytest.MonkeyPatch,
):
    other = tmp_path / 'other'
    other.mkdir()
    monkeypatch.setenv('PATH', str(other))
    fake_executable(other)
    configured = fake_executable(tmp_path)
    configure(configured)
    assert binary.find_executable() == configured


def test_falls_back_to_path(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    found = fake_executable(tmp_path)
    monkeypatch.setenv('PATH', str(tmp_path))
    assert binary.find_executable() == found


def test_missing_executable_explains_the_setting(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setenv('PATH', str(tmp_path))
    with pytest.raises(ExecutableError, match='Processing'):
        binary.find_executable()


def test_configured_path_must_exist(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    configure(tmp_path / 'missing')
    with pytest.raises(ExecutableError, match='does not exist'):
        binary.find_executable()


def test_rules_are_read_from_the_executable(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    configure(fake_executable(tmp_path))
    rules = binary.rules()
    assert [rule.name for rule in rules] == [rule['name'] for rule in RULES]
    dangles, covered = rules
    assert dangles.group is Group.LINE
    assert dangles.slug == 'line_must_not_have_dangles'
    assert dangles.display_name == 'Lines must not have dangles'
    assert dangles.other is None
    assert covered.other == 'line'


def test_old_executable_without_layer_kinds_is_rejected(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    old = [{'name': 'line/must-not-overlap', 'description': 'x'}]
    configure(fake_executable(tmp_path, old))
    with pytest.raises(ExecutableError, match='too old'):
        binary.rules()


def test_failing_executable_reports_its_error(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    script = tmp_path / 'topology-checker'
    script.write_text('#!/bin/sh\necho \'{"error": "broken"}\' >&2\nexit 2\n')
    script.chmod(0o755)
    configure(script)
    with pytest.raises(ExecutableError, match='broken'):
        binary.rules()
