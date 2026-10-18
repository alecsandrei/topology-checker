"""What the plugin does with the programs users point it at."""

from __future__ import annotations

import collections.abc as c
import os
import signal
import sys
import time
import types
from pathlib import Path

import pytest

from topology_rules.core import binary
from topology_rules.exceptions import ExecutableError

from .helpers import RULES, fake_executable

# Programs that are shell scripts, an alarm signal and the descriptors of a
# terminal: the cases are about POSIX programs. The runner flags of Windows
# are tested in test_windows
pytestmark = pytest.mark.skipif(
    sys.platform == 'win32', reason='shell scripts and POSIX signals'
)

DEADLINE_SECONDS = 20


@pytest.fixture(autouse=True)
def deadline() -> c.Iterator[None]:
    """Fails a test that hangs, as QGIS would."""

    def expired(signum: int, frame: types.FrameType | None) -> None:
        raise AssertionError(f'still running after {DEADLINE_SECONDS} s')

    previous = signal.signal(signal.SIGALRM, expired)
    signal.alarm(DEADLINE_SECONDS)
    yield
    signal.alarm(0)
    signal.signal(signal.SIGALRM, previous)


@pytest.fixture
def blocked_stdin() -> c.Iterator[None]:
    """Standard input that never has data nor ends, like a terminal."""
    read, write = os.pipe()
    saved = os.dup(0)
    os.dup2(read, 0)
    yield
    os.dup2(saved, 0)
    for descriptor in (read, write, saved):
        os.close(descriptor)


def script(directory: Path, text: str, name: str = 'topology-checker') -> Path:
    path = directory / name
    path.write_text(text)
    path.chmod(0o755)
    return path


def test_a_program_that_hangs_does_not_freeze_the_plugin(
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    monkeypatch: pytest.MonkeyPatch,
):
    monkeypatch.setattr(binary, 'PROBE_TIMEOUT', 0.3)
    configure(script(tmp_path, '#!/bin/sh\nsleep 600\n'))
    started = time.monotonic()
    with pytest.raises(ExecutableError, match='did not answer'):
        binary.rules()
    assert time.monotonic() - started < 10


def test_a_program_that_reads_the_input_does_not_freeze_the_plugin(
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    blocked_stdin: None,
):
    configure(script(tmp_path, '#!/bin/sh\ncat\n'))
    with pytest.raises(ExecutableError):
        binary.rules()


def test_output_that_is_not_utf8_is_an_error_of_the_executable(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    configure(script(tmp_path, "#!/bin/sh\nprintf '\\377\\376'\n"))
    with pytest.raises(ExecutableError):
        binary.rules()


@pytest.mark.parametrize(
    'text',
    [
        '',
        'not json',
        'null',
        '"text"',
        '{}',
        '{"name": "line/x"}',
        '[1, 2]',
        '[null]',
        '[]',
    ],
)
def test_output_that_is_not_a_list_of_rules(
    tmp_path: Path, configure: c.Callable[[str | Path], None], text: str
):
    configure(script(tmp_path, f"#!/bin/sh\ncat <<'EOF'\n{text}\nEOF\n"))
    with pytest.raises(ExecutableError):
        binary.rules()


def test_a_rule_of_a_group_the_plugin_does_not_know_is_skipped(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    unknown = {**RULES[0], 'name': 'raster/must-not-have-holes'}
    configure(fake_executable(tmp_path, [unknown, *RULES]))
    assert [rule.name for rule in binary.rules()] == [
        rule['name'] for rule in RULES
    ]


@pytest.mark.parametrize('name', ['dangles', 'line/a/b', '', '/'])
def test_a_rule_name_of_another_shape_is_skipped(
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    name: str,
):
    odd = {**RULES[0], 'name': name}
    configure(fake_executable(tmp_path, [odd, *RULES]))
    assert [rule.name for rule in binary.rules()] == [
        rule['name'] for rule in RULES
    ]


def test_extra_fields_are_ignored(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    extra = [{**rule, 'since': '9.9'} for rule in RULES]
    configure(fake_executable(tmp_path, extra))
    assert len(binary.rules()) == len(RULES)


def test_a_path_pasted_with_quotes(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    path = fake_executable(tmp_path)
    configure(f'"{path}"')
    assert binary.find_executable() == path


def test_a_path_with_spaces_around(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    path = fake_executable(tmp_path)
    configure(f'  {path}\n')
    assert binary.find_executable() == path


def test_a_path_in_the_home_folder(
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    monkeypatch: pytest.MonkeyPatch,
):
    fake_executable(tmp_path)
    monkeypatch.setenv('HOME', str(tmp_path))
    configure('~/topology-checker')
    assert binary.find_executable() == tmp_path / 'topology-checker'


def test_a_folder_instead_of_the_program(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    fake_executable(tmp_path)
    configure(tmp_path)
    with pytest.raises(ExecutableError, match=r'folder|not a file|exist'):
        binary.rules()


def test_a_file_that_cannot_be_run(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    path = script(tmp_path, '#!/bin/sh\n')
    path.chmod(0o644)
    configure(path)
    with pytest.raises(ExecutableError):
        binary.rules()


def test_a_script_with_a_missing_interpreter(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    configure(script(tmp_path, '#!/nonexistent/interpreter\n'))
    with pytest.raises(ExecutableError):
        binary.rules()


def test_a_link_to_the_program(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    real = fake_executable(tmp_path)
    link = tmp_path / 'link'
    link.symlink_to(real)
    configure(link)
    assert len(binary.rules()) == len(RULES)


def test_a_replaced_program_is_read_again(
    tmp_path: Path, configure: c.Callable[[str | Path], None]
):
    path = fake_executable(tmp_path, RULES[:1])
    configure(path)
    assert len(binary.rules()) == 1
    time.sleep(0.01)
    fake_executable(tmp_path, RULES)
    assert len(binary.rules()) == len(RULES)


def test_a_program_that_vanishes_after_it_was_found(
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    monkeypatch: pytest.MonkeyPatch,
):
    path = fake_executable(tmp_path)
    configure(path)
    original = binary.find_executable

    def vanishing() -> Path:
        found = original()
        found.unlink()
        return found

    monkeypatch.setattr(binary, 'find_executable', vanishing)
    with pytest.raises(ExecutableError):
        binary.rules()
