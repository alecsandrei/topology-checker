"""What the plugin does on Windows, where most of its users are: tested here
with the Windows path rules (`ntpath`) and a patched platform, as no run on
Windows is at hand.

The seams that these tests rely on: `commands.shown_path` reads `os.path` and
`os.sep` through the `os` module of `commands`, and `runner` reads
`sys.platform` and `subprocess.Popen` when a command runs."""

from __future__ import annotations

import collections.abc as c
import ntpath
import os
import shutil
import subprocess
import sys
import types
import typing as t
from pathlib import Path, PureWindowsPath

import pytest
import tomllib

from topology_rules.core import binary, commands, runner
from topology_rules.core.binary import Rule
from topology_rules.core.commands import Check, shown_path, write_config
from topology_rules.core.definition import (
    DefinitionError,
    Imported,
    read_definition,
)
from topology_rules.core.layers import Dataset
from topology_rules.exceptions import ExecutableError, RunError
from topology_rules.panel import exchange
from topology_rules.panel.entries import Entry
from topology_rules.panel.exchange import export_definition

from .helpers import saved_layer, write_toml

CREATE_NO_WINDOW = 0x08000000
CREATE_NEW_PROCESS_GROUP = 0x00000200
DANGLES = Rule('line/must-not-have-dangles', 'x', 'line', None)
COVERED = Rule('point/must-be-covered-by-lines', 'x', 'point', 'line')
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
WINDOWS_PATHS = [
    'C:\\data\\roads.gpkg',
    'C:/data/roads.gpkg',
    '\\\\server\\share\\a.gpkg',
    '\\\\server\\share\\sub folder\\a.gpkg',
    'C:\\Program Files\\Data\\my roads.gpkg',
    'D:\\Données\\route ü 日本.gpkg',
    'C:\\data\\"quoted"\\roads.gpkg',
    'C:\\data\\',
]


def windows_os() -> types.SimpleNamespace:
    """The `os` of commands as it is on Windows."""
    return types.SimpleNamespace(path=ntpath, sep='\\')


@pytest.fixture
def windows(monkeypatch: pytest.MonkeyPatch) -> None:
    """The platform is Windows, for the code that asks."""
    monkeypatch.setattr(sys, 'platform', 'win32')
    monkeypatch.setattr(
        subprocess, 'CREATE_NO_WINDOW', CREATE_NO_WINDOW, raising=False
    )
    monkeypatch.setattr(
        subprocess,
        'CREATE_NEW_PROCESS_GROUP',
        CREATE_NEW_PROCESS_GROUP,
        raising=False,
    )


@pytest.fixture
def ntfs(monkeypatch: pytest.MonkeyPatch) -> None:
    """The paths of commands are those of Windows."""
    monkeypatch.setattr(commands, 'os', windows_os())


# The executable


def program(directory: Path) -> Path:
    """A file that is named as the program of Windows is, which only has to
    be there to be found."""
    path = directory / 'topology-checker.exe'
    path.write_text('')
    # `which` wants a program that can be run, on the system that tests this
    path.chmod(0o755)
    return path


def test_a_path_without_exe_finds_the_exe_on_windows(
    windows: None,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
):
    program(tmp_path)
    configure(tmp_path / 'topology-checker')
    try:
        answer = str(binary.find_executable())
    except ExecutableError as error:
        # Saying that the file needs its extension is also an answer
        answer = str(error)
    assert answer.endswith('topology-checker.exe') or '.exe' in answer
    assert not answer.endswith('topology-checker')


def test_a_path_without_exe_is_not_guessed_elsewhere(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    program(tmp_path)
    configure(tmp_path / 'topology-checker')
    with pytest.raises(ExecutableError, match='does not exist'):
        binary.find_executable()


def test_the_exe_on_the_path_is_found_through_pathext(
    windows: None, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    program(tmp_path)
    monkeypatch.setenv('PATH', str(tmp_path))
    # In lower case for a file system that tells the case, and split as this
    # system splits a path list
    monkeypatch.setenv('PATHEXT', os.pathsep.join(['.com', '.exe', '.bat']))
    # `which` asks Windows whether the current folder is searched
    monkeypatch.setattr(
        shutil,
        '_winapi',
        types.SimpleNamespace(NeedCurrentDirectoryForExePath=lambda _: False),
        raising=False,
    )
    assert binary.find_executable().name == 'topology-checker.exe'


def test_a_path_with_the_exe_is_taken_as_it_is(
    windows: None,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
):
    path = program(tmp_path)
    configure(path)
    assert binary.find_executable() == path


@pytest.mark.parametrize(
    'pasted',
    [
        '"C:\\Program Files\\topology-checker.exe" ',
        ' "C:\\Program Files\\topology-checker.exe"',
        "'C:\\Program Files\\topology-checker.exe'",
        '  " C:\\Program Files\\topology-checker.exe "  ',
        'C:\\Program Files\\topology-checker.exe\r\n',
    ],
)
def test_a_pasted_windows_path_is_cleaned(
    configure: c.Callable[[str | Path], None], pasted: str
):
    configure(pasted)
    assert binary.configured_path() == 'C:\\Program Files\\topology-checker.exe'


def test_a_pasted_path_with_spaces_and_quotes_finds_the_file(
    windows: None,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
):
    folder = tmp_path / 'Program Files'
    folder.mkdir()
    path = program(folder)
    configure(f'"{path}" ')
    assert binary.find_executable() == path


def test_a_quote_inside_the_path_is_kept(
    configure: c.Callable[[str | Path], None],
):
    configure('C:\\a"b\\topology-checker.exe')
    assert binary.configured_path() == 'C:\\a"b\\topology-checker.exe'


# The runner


class Pipe:
    def __init__(self) -> None:
        self.closed = False

    def close(self) -> None:
        self.closed = True


class FakeProcess:
    """A process that answers at once, or after one wait that times out, and
    that is ended by being killed, unless something outlives it and holds
    its pipes."""

    def __init__(self, hangs: bool = False, outlived: bool = False) -> None:
        self.hangs = hangs
        self.outlived = outlived
        self.returncode = 0
        self.pid = 4242
        self.killed = False
        self.stdout = Pipe()
        self.stderr = Pipe()

    def communicate(self, timeout: float | None = None) -> tuple[str, str]:
        if (self.outlived or (self.hangs and not self.killed)) and (
            timeout is not None
        ):
            raise subprocess.TimeoutExpired('tc', timeout)
        return '', ''

    def kill(self) -> None:
        self.killed = True

    def wait(self, timeout: float | None = None) -> int:
        return 0


class Canceled:
    def isCanceled(self) -> bool:
        return True


@pytest.fixture
def popen(
    monkeypatch: pytest.MonkeyPatch,
) -> list[dict[str, t.Any]]:
    """Records how a command is started, without starting it."""
    calls: list[dict[str, t.Any]] = []

    def start(command: list[str], **kwargs: t.Any) -> FakeProcess:
        calls.append({'command': command, **kwargs})
        return FakeProcess()

    monkeypatch.setattr(subprocess, 'Popen', start)
    return calls


def test_a_command_on_windows_opens_no_console_window(
    windows: None, popen: list[dict[str, t.Any]]
):
    runner.run(['tc.exe', '--json', 'rules'])
    assert popen[0]['creationflags'] & CREATE_NO_WINDOW


def test_a_command_on_windows_has_a_process_group_of_its_own(
    windows: None, popen: list[dict[str, t.Any]]
):
    # So that the tree of processes of a tool behind a launcher can be ended
    runner.run(['tc.exe', '--json', 'rules'])
    assert popen[0]['creationflags'] == (
        CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
    )


def test_a_command_on_windows_does_not_start_a_session(
    windows: None, popen: list[dict[str, t.Any]]
):
    runner.run(['tc.exe', '--json', 'rules'])
    assert popen[0]['start_new_session'] is False


def test_a_command_elsewhere_has_no_creation_flags(
    popen: list[dict[str, t.Any]],
):
    runner.run(['tc', '--json', 'rules'])
    assert popen[0]['creationflags'] == 0
    assert popen[0]['start_new_session'] is True


def test_a_command_on_windows_has_no_shell_and_no_input(
    windows: None, popen: list[dict[str, t.Any]]
):
    runner.run(['tc.exe', '--json', 'rules'])
    assert not popen[0].get('shell')
    assert popen[0]['stdin'] == subprocess.DEVNULL
    assert isinstance(popen[0]['command'], list)


def test_a_program_that_is_not_there_is_a_run_error_on_windows(
    windows: None, monkeypatch: pytest.MonkeyPatch
):
    def missing(command: list[str], **kwargs: t.Any) -> None:
        raise FileNotFoundError(2, 'The system cannot find the file')

    monkeypatch.setattr(subprocess, 'Popen', missing)
    with pytest.raises(RunError, match='Could not run'):
        runner.run(['C:\\nowhere\\tc.exe'])


@pytest.fixture
def taskkill(monkeypatch: pytest.MonkeyPatch) -> list[dict[str, t.Any]]:
    """Records the commands that the runner runs to end a tree of processes
    on Windows."""
    calls: list[dict[str, t.Any]] = []

    def run(command: list[str], **kwargs: t.Any) -> object:
        calls.append({'command': command, **kwargs})
        return types.SimpleNamespace(returncode=0)

    monkeypatch.setattr(subprocess, 'run', run)
    return calls


def test_canceling_on_windows_kills_the_process(
    windows: None,
    monkeypatch: pytest.MonkeyPatch,
    taskkill: list[dict[str, t.Any]],
):
    process = FakeProcess(hangs=True)
    monkeypatch.setattr(subprocess, 'Popen', lambda *a, **k: process)
    with pytest.raises(RunError, match='Canceled'):
        runner.run(['tc.exe'], Canceled())  # type: ignore[arg-type]
    assert process.killed


def test_canceling_on_windows_ends_the_tree_of_processes(
    windows: None,
    monkeypatch: pytest.MonkeyPatch,
    taskkill: list[dict[str, t.Any]],
):
    process = FakeProcess(hangs=True)
    monkeypatch.setattr(subprocess, 'Popen', lambda *a, **k: process)
    with pytest.raises(RunError, match='Canceled'):
        runner.run(['tc.exe'], Canceled())  # type: ignore[arg-type]
    (call,) = taskkill
    assert call['command'] == ['taskkill', '/T', '/F', '/PID', '4242']
    assert call['creationflags'] == CREATE_NO_WINDOW


def test_a_tool_that_takes_too_long_on_windows_is_ended_with_its_tree(
    windows: None,
    monkeypatch: pytest.MonkeyPatch,
    taskkill: list[dict[str, t.Any]],
):
    process = FakeProcess(hangs=True)
    monkeypatch.setattr(subprocess, 'Popen', lambda *a, **k: process)
    with pytest.raises(RunError, match='did not answer'):
        runner.run(['tc.exe'], timeout=-1)
    assert [call['command'][:3] for call in taskkill] == [
        ['taskkill', '/T', '/F']
    ]
    assert process.killed


def test_a_program_that_outlives_its_tool_does_not_hold_up_a_cancel(
    windows: None,
    monkeypatch: pytest.MonkeyPatch,
    taskkill: list[dict[str, t.Any]],
):
    # What the tool started holds the pipes of the tool open after the kill
    process = FakeProcess(hangs=True, outlived=True)
    monkeypatch.setattr(subprocess, 'Popen', lambda *a, **k: process)
    with pytest.raises(RunError, match='Canceled'):
        runner.run(['tc.exe'], Canceled())  # type: ignore[arg-type]
    assert process.stdout.closed
    assert process.stderr.closed


def test_canceling_elsewhere_ends_the_group_of_the_tool(
    monkeypatch: pytest.MonkeyPatch,
):
    ended: list[tuple[int, int]] = []
    process = FakeProcess(hangs=True)
    monkeypatch.setattr(subprocess, 'Popen', lambda *a, **k: process)
    monkeypatch.setattr(
        runner.os,
        'killpg',
        lambda number, signal: ended.append((number, signal)),
    )
    with pytest.raises(RunError, match='Canceled'):
        runner.run(['tc'], Canceled())  # type: ignore[arg-type]
    assert ended == [(4242, runner.signal.SIGKILL)]


# The definition file


@pytest.mark.parametrize('source', WINDOWS_PATHS)
def test_a_windows_path_survives_the_toml(tmp_path: Path, source: str):
    path = tmp_path / 'checks.toml'
    write_config(path, [Check(DANGLES, Dataset(source, 'roads'))])
    config = tomllib.loads(path.read_bytes().decode('utf-8'))
    assert config['layers']['roads']['path'] == source


@pytest.mark.parametrize('source', WINDOWS_PATHS)
def test_a_windows_path_survives_the_toml_without_a_layer_name(
    tmp_path: Path, source: str
):
    path = tmp_path / 'checks.toml'
    aliases = write_config(path, [Check(DANGLES, Dataset(source))])
    config = tomllib.loads(path.read_bytes().decode('utf-8'))
    (alias,) = aliases.values()
    assert config['layers'][alias] == {'path': source}


def test_a_layer_name_with_unicode_survives_the_toml(tmp_path: Path):
    path = tmp_path / 'checks.toml'
    name = 'rue « Éléonore » 日本'
    write_config(path, [Check(DANGLES, Dataset('C:\\data\\a.gpkg', name))])
    config = tomllib.loads(path.read_bytes().decode('utf-8'))
    assert config['layers'][name]['layer'] == name
    assert config['rules'][0]['layer'] == name


def test_the_definition_is_written_as_utf8(tmp_path: Path):
    path = tmp_path / 'checks.toml'
    write_config(path, [Check(DANGLES, Dataset('D:\\Données\\a.gpkg', 'ü'))])
    assert 'Données'.encode() in path.read_bytes()


def test_exported_rules_keep_windows_paths(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    sources = {
        'a': Dataset('\\\\server\\share\\sub folder\\a.gpkg', 'roads'),
        'b': Dataset('D:\\Données\\route ü.gpkg', 'stops'),
    }
    monkeypatch.setattr(
        exchange, 'dataset_of', lambda layer_id: sources.get(layer_id or '')
    )
    monkeypatch.setattr(commands, 'os', windows_os())
    entries = [Entry(COVERED, 'b', 'stops', 'm', 'a', 'roads')]
    path = tmp_path / 'topology.toml'
    export_definition(entries, None, path)
    config = tomllib.loads(path.read_text(encoding='utf-8'))
    assert {k: v['path'] for k, v in config['layers'].items()} == {
        'stops': 'D:\\Données\\route ü.gpkg',
        'roads': '\\\\server\\share\\sub folder\\a.gpkg',
    }


# Relative paths


def shown(source: str, config: str) -> str:
    return shown_path(source, t.cast('Path', PureWindowsPath(config)), True)


def test_a_path_on_the_same_drive_is_shown_relative(ntfs: None):
    assert shown('D:\\data\\roads.gpkg', 'D:\\proj\\topology.toml') == (
        '../data/roads.gpkg'
    )


def test_a_path_in_the_folder_of_the_file_is_shown_relative(ntfs: None):
    assert shown('D:\\proj\\data\\roads.gpkg', 'D:\\proj\\topology.toml') == (
        'data/roads.gpkg'
    )


def test_the_drive_letter_may_differ_in_case(ntfs: None):
    assert shown('d:\\data\\roads.gpkg', 'D:\\proj\\topology.toml') == (
        '../data/roads.gpkg'
    )


def test_a_path_on_another_drive_stays_absolute(ntfs: None):
    source = 'C:\\data\\roads.gpkg'
    assert shown(source, 'D:\\proj\\topology.toml') == source


def test_a_share_and_a_drive_stay_absolute(ntfs: None):
    source = '\\\\server\\share\\a.gpkg'
    assert shown(source, 'D:\\proj\\topology.toml') == source


def test_a_drive_and_a_share_stay_absolute(ntfs: None):
    source = 'D:\\data\\a.gpkg'
    assert shown(source, '\\\\server\\share\\proj\\topology.toml') == source


def test_two_shares_stay_absolute(ntfs: None):
    source = '\\\\server\\one\\a.gpkg'
    assert shown(source, '\\\\server\\two\\topology.toml') == source


def test_a_path_on_the_same_share_is_shown_relative(ntfs: None):
    assert shown(
        '\\\\server\\share\\data\\a.gpkg',
        '\\\\server\\share\\proj\\topology.toml',
    ) == ('../data/a.gpkg')


def test_a_relative_path_never_has_backslashes(ntfs: None):
    shown_text = shown('D:\\a b\\ü\\roads.gpkg', 'D:\\proj\\sub\\topology.toml')
    assert '\\' not in shown_text


def test_writing_data_on_another_drive_does_not_fail(
    tmp_path: Path, ntfs: None
):
    path = tmp_path / 'topology.toml'
    source = 'C:\\data\\roads.gpkg'
    write_config(
        path, [Check(DANGLES, Dataset(source, 'roads'))], relative=True
    )
    config = tomllib.loads(path.read_text(encoding='utf-8'))
    assert config['layers']['roads']['path'] == source


def test_exporting_data_on_another_drive_does_not_fail(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    source = Dataset('C:\\data\\roads.gpkg', 'roads')
    monkeypatch.setattr(exchange, 'dataset_of', lambda layer_id: source)
    monkeypatch.setattr(commands, 'os', windows_os())
    path = tmp_path / 'topology.toml'
    export_definition([Entry(DANGLES, 'a', 'roads', 'm')], None, path)
    config = tomllib.loads(path.read_text(encoding='utf-8'))
    assert config['layers']['roads']['path'] == source.path


@pytest.mark.parametrize(
    'source', ['C:\\a.gpkg', '\\\\h\\s\\a.gpkg', 'Z:\\b\\c.gpkg']
)
def test_exporting_keeps_the_path_of_any_other_drive(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, source: str
):
    monkeypatch.setattr(
        exchange, 'dataset_of', lambda layer_id: Dataset(source, 'roads')
    )
    monkeypatch.setattr(commands, 'os', windows_os())
    path = tmp_path / 'topology.toml'
    export_definition([Entry(DANGLES, 'a', 'roads', 'm')], None, path)
    config = tomllib.loads(path.read_text(encoding='utf-8'))
    assert config['layers']['roads']['path'] == source


# Reading relative paths


@pytest.fixture
def definition(tmp_path: Path, executable: str) -> c.Callable[[str], Imported]:
    """Reads a definition on roads that is saved at data/roads.gpkg, with the
    path to it as the file would write it."""
    (tmp_path / 'data').mkdir()
    saved_layer(tmp_path / 'data', CHAIN, 'roads')
    known = {rule.name: rule for rule in binary.rules()}

    def read(written: str) -> Imported:
        toml = write_toml(
            tmp_path / 'topology.toml',
            {'roads': (written, 'roads')},
            [('line/must-not-have-dangles', 'roads', None)],
        )
        return read_definition(toml, known)

    return read


@pytest.mark.parametrize(
    'written',
    [
        'data/roads.gpkg',
        'data\\roads.gpkg',
        './data/roads.gpkg',
        '.\\data\\roads.gpkg',
        'data/../data/roads.gpkg',
        'data\\..\\data\\roads.gpkg',
        'data\\sub\\..\\roads.gpkg',
    ],
)
def test_a_relative_path_is_read_with_either_separator(
    definition: c.Callable[[str], Imported], written: str, tmp_path: Path
):
    (tmp_path / 'data' / 'sub').mkdir(exist_ok=True)
    imported = definition(written)
    assert len(imported.rules) == 1
    layer = imported.rules[0][1]
    assert layer.isValid()
    assert (
        Path(layer.source().split('|')[0]).resolve()
        == (tmp_path / 'data' / 'roads.gpkg').resolve()
    )


def test_a_missing_file_with_backslashes_is_reported(
    definition: c.Callable[[str], Imported],
):
    with pytest.raises(DefinitionError, match='missing'):
        definition('data\\nothing.gpkg')
