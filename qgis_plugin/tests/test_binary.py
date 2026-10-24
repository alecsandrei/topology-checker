"""Finding the executable and reading the rules that it lists."""

from __future__ import annotations

import collections.abc as c
import sys
from pathlib import Path

import pytest

from topology_rules.core import binary
from topology_rules.enums import Group
from topology_rules.exceptions import ExecutableError, MissingError

from .helpers import RULES, fake_executable, write_tool


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


@pytest.mark.skipif(sys.platform == 'win32', reason='a shell script')
def test_failing_executable_reports_its_error(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    script = tmp_path / 'topology-checker'
    script.write_text('#!/bin/sh\necho \'{"error": "broken"}\' >&2\nexit 2\n')
    script.chmod(0o755)
    configure(script)
    with pytest.raises(ExecutableError, match='broken'):
        binary.rules()


# Why the executable cannot be used, translated

CHOOSE = (
    "<Choose a newer build with 'Choose executable…' or under "
    'Settings > Options > Processing > Providers > Topology Rules.>'
)


@pytest.mark.usefixtures('marked')
def test_why_the_configured_path_cannot_be_used_is_translated(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    configure(tmp_path)
    with pytest.raises(ExecutableError) as folder:
        binary.find_executable()
    missing = tmp_path / 'missing'
    configure(missing)
    with pytest.raises(ExecutableError) as absent:
        binary.find_executable()
    assert str(folder.value) == (
        f'<This is a folder, not the topology-checker executable: {tmp_path}>'
    )
    assert str(absent.value) == (
        f'<The topology-checker executable does not exist: {missing}>'
    )


@pytest.mark.usefixtures('marked')
def test_why_no_executable_is_found_is_translated(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setenv('PATH', str(tmp_path))
    with pytest.raises(MissingError) as raised:
        binary.find_executable()
    assert str(raised.value).startswith('<topology-checker was not found. ')
    assert str(raised.value).endswith(' > Topology Rules.>')


@pytest.mark.usefixtures('marked')
@pytest.mark.parametrize(
    ('text', 'path', 'message'),
    [
        (
            '{}',
            'tool',
            '<<The topology-checker at tool> is too old for the plugin, '
            'which needs topology-checker 0.2.0 or newer: it does not list '
            'the layers each rule needs. {choose}>',
        ),
        ('[]', '', '<<This topology-checker> lists no rules. {choose}>'),
    ],
    ids=['too old', 'no rules'],
)
def test_why_the_rules_cannot_be_read_is_translated(
    text: str, path: str, message: str
):
    with pytest.raises(ExecutableError) as raised:
        binary.parse_rules(text, path)
    assert str(raised.value) == message.format(choose=CHOOSE)


@pytest.mark.usefixtures('marked')
def test_why_the_executable_does_not_work_is_translated(tmp_path: Path):
    tool = write_tool(
        tmp_path / 'topology-checker',
        '#!/usr/bin/env python3\n'
        'import sys\n'
        'print(\'{"error": "broken"}\', file=sys.stderr)\n'
        'sys.exit(2)\n',
    )
    with pytest.raises(ExecutableError) as broken:
        binary.Probe.ask(str(tool))
    gone = tmp_path / 'gone'
    with pytest.raises(ExecutableError) as unstarted:
        binary.Probe.ask(str(gone))
    assert str(broken.value) == (
        f'<<The topology-checker at {tool}> does not work: broken {CHOOSE}>'
    )
    assert str(unstarted.value).endswith(f' {CHOOSE}')


@pytest.mark.usefixtures('marked')
def test_why_the_executable_cannot_be_read_is_translated(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    gone = tmp_path / 'gone'
    monkeypatch.setattr(binary, 'find_executable', lambda: gone)
    with pytest.raises(ExecutableError) as raised:
        binary.locate()
    assert str(raised.value).startswith(f'<Could not read {gone}: ')
    assert str(raised.value).endswith('>')
