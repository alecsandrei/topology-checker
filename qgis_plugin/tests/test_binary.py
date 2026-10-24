"""Finding the executable and reading the rules that it lists."""

from __future__ import annotations

import collections.abc as c
import os
import shutil
import sys
from pathlib import Path

import pytest

from topology_rules.core import binary
from topology_rules.enums import EXECUTABLE_NAME, Group
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
    with pytest.raises(ExecutableError, match='Processing') as raised:
        binary.find_executable()
    assert "'cargo install topology-checker --locked'" in str(raised.value)


# Cargo's bin folder


@pytest.fixture
def home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A home folder of the test, no tool on the PATH, and none of the
    variables of Cargo, so that its bin folder is `~/.cargo/bin` here."""
    folder = tmp_path / 'home'
    folder.mkdir()
    monkeypatch.setenv('PATH', str(tmp_path / 'nothing'))
    # The home folder of POSIX, and the one of Windows
    monkeypatch.setenv('HOME', str(folder))
    monkeypatch.setenv('USERPROFILE', str(folder))
    monkeypatch.delenv('CARGO_INSTALL_ROOT', raising=False)
    monkeypatch.delenv('CARGO_HOME', raising=False)
    return folder


def program(folder: Path, name: str = EXECUTABLE_NAME) -> Path:
    """A file that can be run, in a folder that is made for it."""
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / name
    path.write_text('')
    path.chmod(0o755)
    return path


def cargo_program(root: Path) -> Path:
    """The program as `cargo install` puts it in the bin folder of the
    root."""
    name = EXECUTABLE_NAME
    if sys.platform == 'win32':
        name = f'{name}.exe'
    return program(root / 'bin', name)


def test_the_setting_wins_over_cargo(
    home: Path, tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    cargo_program(home / '.cargo')
    configured = fake_executable(tmp_path)
    configure(configured)
    assert binary.find_executable() == configured


def test_the_path_wins_over_cargo(
    home: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    cargo_program(home / '.cargo')
    found = fake_executable(tmp_path)
    monkeypatch.setenv('PATH', str(tmp_path))
    assert binary.find_executable() == found


@pytest.mark.parametrize(
    ('variables', 'root'),
    [
        ({'CARGO_INSTALL_ROOT': 'root', 'CARGO_HOME': 'cargo'}, 'root'),
        ({'CARGO_HOME': 'cargo'}, 'cargo'),
        ({}, 'home/.cargo'),
        ({'CARGO_INSTALL_ROOT': '', 'CARGO_HOME': ''}, 'home/.cargo'),
    ],
    ids=['install root', 'cargo home', 'home', 'empty variables'],
)
def test_cargo_bin_is_the_one_that_cargo_installs_to(
    home: Path,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    variables: dict[str, str],
    root: str,
):
    made = {
        folder: cargo_program(tmp_path / folder)
        for folder in ('root', 'cargo', 'home/.cargo')
    }
    for name, value in variables.items():
        monkeypatch.setenv(name, str(tmp_path / value) if value else '')
    assert binary.find_executable() == made[root]


def test_nothing_in_cargo_bin_is_not_found(home: Path):
    (home / '.cargo' / 'bin').mkdir(parents=True)
    with pytest.raises(MissingError):
        binary.find_executable()


@pytest.mark.skipif(sys.platform == 'win32', reason='any file can be run')
def test_a_file_in_cargo_bin_that_cannot_run_is_skipped(home: Path):
    cargo_program(home / '.cargo').chmod(0o644)
    with pytest.raises(MissingError):
        binary.find_executable()


def test_a_folder_in_cargo_bin_is_skipped(home: Path):
    (home / '.cargo' / 'bin' / EXECUTABLE_NAME).mkdir(parents=True)
    with pytest.raises(MissingError):
        binary.find_executable()


def test_cargo_bin_has_the_exe_on_windows(
    home: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr(sys, 'platform', 'win32')
    # Nothing on the PATH, which `which` would look up as Windows does
    monkeypatch.setattr(shutil, 'which', lambda *args, **kwargs: None)
    folder = home / '.cargo' / 'bin'
    program(folder)
    with pytest.raises(MissingError):
        binary.find_executable()
    exe = program(folder, f'{EXECUTABLE_NAME}.exe')
    assert binary.find_executable() == exe


# The PATH of the run, before a test replaces it
PATH = os.environ.get('PATH', '')


def path_without_the_tool() -> str:
    """The PATH without the folders that have the tool, which keeps the
    Python that runs the fake tools."""
    return os.pathsep.join(
        folder
        for folder in PATH.split(os.pathsep)
        if folder and shutil.which(EXECUTABLE_NAME, path=folder) is None
    )


@pytest.mark.skipif(sys.platform == 'win32', reason='a script as the program')
def test_an_old_tool_in_cargo_bin_is_too_old(
    home: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setenv('PATH', path_without_the_tool())
    folder = home / '.cargo' / 'bin'
    folder.mkdir(parents=True)
    old = [{'name': 'line/must-not-overlap', 'description': 'x'}]
    path = fake_executable(folder, old)
    with pytest.raises(ExecutableError, match='too old') as raised:
        binary.rules()
    assert str(raised.value).startswith(f'The topology-checker at {path} ')


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
