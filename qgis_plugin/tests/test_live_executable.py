"""The executable with --progress: what it says is read as it says it, from
both pipes at once, whatever bytes and line ends it uses, and a canceled
run ends it at once. A tool that does not know the flag runs as before, and
is asked again once it is replaced."""

from __future__ import annotations

import contextlib
import os
import signal
import subprocess
import sys
import time
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsProcessingFeedback

from topology_rules.core import commands, runner
from topology_rules.core.progress import Stage
from topology_rules.exceptions import RunError

from .helpers import process_alive, wait_for, write_tool
from .live import (
    forwarding_executable,
    let_all_lines,
    let_end,
    refusing_executable,
    replaying_executable,
    run_told,
)
from .test_live_engines import file_rows, jobs_of, lines

not_on_windows = pytest.mark.skipif(
    sys.platform == 'win32', reason='a test of the sessions of POSIX'
)

# The start of each tool here: it says bytes on its error output, and
# waits for the files that the test makes
PRELUDE = """#!/usr/bin/env python3
import os
import subprocess
import sys
import time


def say(data):
    sys.stderr.buffer.write(data)
    sys.stderr.buffer.flush()


def wait(path):
    end = time.monotonic() + 120
    while not os.path.exists(path):
        if time.monotonic() > end:
            sys.exit(3)
        time.sleep(0.01)


def tell(path, text):
    with open(path + '.part', 'w') as file:
        file.write(text)
    os.replace(path + '.part', path)
"""


def tool(directory: Path, body: str) -> str:
    directory.mkdir(exist_ok=True)
    return str(write_tool(directory / 'tool', PRELUDE + body))


def numbers(path: Path) -> list[int]:
    """The process numbers that a tool wrote down."""
    wait_for(path.exists)
    return [int(number) for number in path.read_text().split()]


@pytest.fixture(autouse=True)
def asked_afresh(monkeypatch: pytest.MonkeyPatch) -> None:
    """No tool is known to refuse --progress before the test."""
    monkeypatch.setattr(commands, 'WITHOUT_PROGRESS', set())


# Reading what the tool says


def test_each_line_is_read_as_soon_as_the_tool_says_it(tmp_path: Path):
    """The tool says its next line only once the test has read the last:
    a run that read the lines at the end would wait for ever."""
    gates = tmp_path / 'gates'
    gates.mkdir()
    command = tool(
        tmp_path / 'tool',
        f"""
for number in range(3):
    say(f'line {{number}}\\n'.encode())
    wait(os.path.join({str(gates)!r}, str(number)))
print('out')
""",
    )
    seen: list[str] = []

    def take(line: str) -> bool:
        seen.append(line)
        (gates / str(len(seen) - 1)).touch()
        return True

    completed = runner.run([command], QgsProcessingFeedback(), lines=take)
    assert completed.exit_code == 0
    assert seen == ['line 0', 'line 1', 'line 2']
    assert completed.stdout == 'out\n'
    assert completed.stderr == ''


def test_both_pipes_are_read_while_the_tool_fills_them(tmp_path: Path):
    """Far more than a pipe holds goes to each, in turns."""
    command = tool(
        tmp_path / 'tool',
        """
for number in range(2000):
    sys.stdout.write('o' * 1000 + '\\n')
    say(f'e{number}\\n'.encode())
sys.stdout.flush()
""",
    )
    taken: list[str] = []

    def take(line: str) -> bool:
        if int(line[1:]) % 2 == 0:
            taken.append(line)
            return True
        return False

    # A run that read one pipe only would stop here
    completed = runner.run([command], timeout=60, lines=take)
    assert completed.exit_code == 0
    assert completed.stdout == ('o' * 1000 + '\n') * 2000
    assert taken == [f'e{n}' for n in range(0, 2000, 2)]
    assert completed.stderr == ''.join(f'e{n}\n' for n in range(1, 2000, 2))


def test_progress_is_read_whatever_else_the_tool_says(tmp_path: Path):
    """Windows line ends, bytes that are not UTF-8, a long line, a line in
    two writes, a line of no rule and a last line with no end."""
    command = tool(
        tmp_path / 'tool',
        """
say(b'reading roads\\r\\n')
say(b'checked rule 1/3 (line/must-not-have-dangles): 3 errors\\r\\n')
say(b'caf\\xe9 \\xff\\xfe\\n')
say(b'x' * 1000000 + b'\\n')
say(b'checked rule ')
say(b'2/3 (line/must-not-intersect): 1 error\\n')
say(b'checked rule 0/0 (nothing): 1 error\\n')
say(b'checked rule 3/3 (line/must-not-self-intersect): 0 errors')
""",
    )
    told: list[tuple[int, int, int]] = []
    take = commands.follower(lambda *done: told.append(done))
    completed = runner.run([command], QgsProcessingFeedback(), lines=take)
    assert completed.exit_code == 0
    assert told == [(0, 3, 3), (1, 3, 1), (2, 3, 0)]
    assert completed.stderr == (
        'reading roads\n'
        + b'caf\xe9 \xff\xfe\n'.decode('utf-8', 'replace')
        + 'x' * 1000000
        + '\n'
        + 'checked rule 0/0 (nothing): 1 error\n'
    )


@pytest.mark.parametrize(
    ('line', 'found'),
    [
        ('checked rule 1/1 (line/must-not-have-dangles): 1 error', (0, 1, 1)),
        ('checked rule 2/8 (polygon/must-not-overlap): 46 errors', (1, 8, 46)),
        (
            'checked rule 8/8 (point/must-be-covered-by-lines): 0 errors',
            (7, 8, 0),
        ),
        ('  checked rule 3/4 (a (b)): 2 errors \r', (2, 4, 2)),
        ('checked rule 0/4 (line/must-not-intersect): 1 error', None),
        ('checked rule 5/4 (line/must-not-intersect): 1 error', None),
        ('checked rule 1/4 (line/must-not-intersect): many errors', None),
        ('checked rule 1/4 line/must-not-intersect: 1 error', None),
        ('rule 1/4 (line/must-not-intersect): 1 error', None),
        ('checked rule 1/4 (line/must-not-intersect): 1 error and more', None),
        ('', None),
    ],
)
def test_what_a_line_of_progress_says(
    line: str, found: tuple[int, int, int] | None
):
    assert commands.progress_of(line) == found


# Canceling


def test_canceling_ends_the_tool_and_what_it_started_at_once(tmp_path: Path):
    pids = tmp_path / 'pids'
    command = tool(
        tmp_path / 'tool',
        f"""
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(600)'])
tell({str(pids)!r}, f'{{os.getpid()}} {{child.pid}}')
say(b'ready\\n')
time.sleep(600)
""",
    )
    feedback = QgsProcessingFeedback()
    canceled: list[float] = []

    def take(line: str) -> bool:
        if line == 'ready':
            feedback.cancel()
            canceled.append(time.monotonic())
        return True

    with pytest.raises(RunError, match='Canceled'):
        runner.run([command], feedback, lines=take)
    # Without waiting for the readers, which see the end of the pipes
    assert time.monotonic() - canceled[0] < runner.STOP_SECONDS
    started, child = numbers(pids)
    wait_for(lambda: not process_alive(started), 30)
    wait_for(lambda: not process_alive(child), 30)


@not_on_windows
def test_a_program_that_outlives_the_tool_does_not_hold_up_a_cancel(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """What the tool started in a session of its own is not ended with it,
    and holds the pipes: the run lets go of them."""
    monkeypatch.setattr(runner, 'STOP_SECONDS', 0.5)
    pids = tmp_path / 'pids'
    command = tool(
        tmp_path / 'tool',
        f"""
child = subprocess.Popen(
    [sys.executable, '-c', 'import time; time.sleep(600)'],
    start_new_session=True,
)
tell({str(pids)!r}, f'{{os.getpid()}} {{child.pid}}')
say(b'ready\\n')
time.sleep(600)
""",
    )
    feedback = QgsProcessingFeedback()
    canceled: list[float] = []

    def take(line: str) -> bool:
        if line == 'ready':
            feedback.cancel()
            canceled.append(time.monotonic())
        return True

    try:
        with pytest.raises(RunError, match='Canceled'):
            runner.run([command], feedback, lines=take)
        assert time.monotonic() - canceled[0] < 3.0
        started, child = numbers(pids)
        wait_for(lambda: not process_alive(started), 30)
        assert process_alive(child)
    finally:
        if pids.exists():
            with contextlib.suppress(ProcessLookupError):
                os.kill(numbers(pids)[1], signal.SIGKILL)


def test_a_tool_that_ends_alone_is_not_held_up_by_its_lines(tmp_path: Path):
    """A tool that says nothing more and ends: the run ends with it."""
    command = tool(tmp_path / 'tool', "say(b'only line\\n')\nsys.exit(4)\n")
    completed = runner.run(
        [command], QgsProcessingFeedback(), lines=lambda line: False
    )
    assert completed == runner.Completed(4, '', 'only line\n')


# A tool from before --progress


def validate_lines(log: Path) -> list[list[str]]:
    return [
        line.split()
        for line in log.read_text().splitlines()
        if 'validate' in line.split()
    ]


def test_a_tool_without_progress_runs_as_before(
    executable: str, tmp_path: Path
):
    log = tmp_path / 'commands'
    (tmp_path / 'old').mkdir()
    old = str(refusing_executable(tmp_path / 'old', log=log))
    jobs = jobs_of(old, lines())
    told = run_told(old, jobs, tmp_path / 'run')
    reference = run_told(executable, jobs, tmp_path / 'reference')
    assert told.outcome.counts == reference.outcome.counts
    assert file_rows(told.output) == file_rows(reference.output)
    # Asked once with the flag, which it refused, then without
    asked = validate_lines(log)
    assert ['--progress' in command for command in asked] == [True, False]
    # It tells no rule, and which rule it checks is never known
    assert told.feedback.checks() == []
    assert [
        stage for stage in told.feedback.stages() if stage[0] is Stage.CHECKING
    ] == [(Stage.CHECKING, None, len(jobs))]
    # The next run does not ask with the flag again
    run_told(old, jobs, tmp_path / 'again')
    assert ['--progress' in command for command in validate_lines(log)] == [
        True,
        False,
        False,
    ]


def test_a_tool_that_is_replaced_at_its_path_is_asked_again(
    executable: str, tmp_path: Path
):
    log = tmp_path / 'commands'
    folder = tmp_path / 'tool'
    folder.mkdir()
    path = refusing_executable(folder, log=log)
    jobs = jobs_of(str(path), lines())
    run_told(str(path), jobs, tmp_path / 'old')
    assert len(commands.WITHOUT_PROGRESS) == 1
    # A newer tool at the same path, which knows the flag
    changed = os.stat(path).st_mtime_ns
    replaced = forwarding_executable(folder, log)
    assert replaced == path
    later = changed + 10**9
    os.utime(path, ns=(later, later))
    told = run_told(str(path), jobs, tmp_path / 'new')
    assert '--progress' in validate_lines(log)[-1]
    assert [done.index for done in told.feedback.checks()] == list(
        range(len(jobs))
    )


def test_lines_of_no_rule_go_to_the_log(executable: str, tmp_path: Path):
    """What the tool says besides its progress is in the log of the run,
    and the rules are told as from a quiet tool."""
    gates = tmp_path / 'gates'
    folder = tmp_path / 'tool'
    folder.mkdir()
    noisy = str(
        replaying_executable(folder, gates, tmp_path / 'started', noise=True)
    )
    let_all_lines(gates)
    let_end(gates)
    jobs = jobs_of(noisy, lines())
    told = run_told(noisy, jobs, tmp_path / 'run')
    reference = run_told(executable, jobs, tmp_path / 'reference')
    assert [d.count for d in told.feedback.checks()] == (
        reference.outcome.counts
    )
    assert file_rows(told.output) == file_rows(reference.output)
    log = told.feedback.textLog()
    assert 'reading roads' in log
    assert b'caf\xe9 \xff\xfe'.decode('utf-8', 'replace') in log
    assert 'x' * 100000 in log
    assert 'checked rule 0/0 (nothing): 1 error' in log


# Windows, with a patched platform and a process that is not started


CREATE_NO_WINDOW = 0x08000000
CREATE_NEW_PROCESS_GROUP = 0x00000200


class Output:
    def __init__(self, text: str = '') -> None:
        self.text = text
        self.closed = False

    def read(self) -> str:
        return self.text

    def close(self) -> None:
        self.closed = True


class ErrorLines:
    def __init__(self, lines: list[str]) -> None:
        self.lines = lines
        self.closed = False

    def __iter__(self) -> t.Iterator[str]:
        return iter(self.lines)

    def close(self) -> None:
        self.closed = True


class FollowedProcess:
    """A process whose pipes have their lines at once, which ends when they
    are read, or runs until it is killed."""

    def __init__(
        self,
        stdout: str = '',
        stderr: list[str] | None = None,
        ends: bool = True,
    ) -> None:
        self.pid = 4242
        self.returncode: int | None = None
        self.ends = ends
        self.killed = False
        self.stdout = Output(stdout)
        self.stderr = ErrorLines(stderr or [])

    def poll(self) -> int | None:
        if self.killed:
            self.returncode = 1
        elif self.ends:
            self.returncode = 0
        return self.returncode

    def kill(self) -> None:
        self.killed = True

    def wait(self, timeout: float | None = None) -> int | None:
        return self.poll()


@pytest.fixture
def windows(monkeypatch: pytest.MonkeyPatch) -> list[list[str]]:
    """The platform is Windows; returns the commands that end trees of
    processes there."""
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
    ended: list[list[str]] = []

    def run(command: list[str], **kwargs: t.Any) -> object:
        ended.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(subprocess, 'run', run)
    return ended


def started_as(
    monkeypatch: pytest.MonkeyPatch, process: FollowedProcess
) -> list[dict[str, t.Any]]:
    calls: list[dict[str, t.Any]] = []

    def start(command: list[str], **kwargs: t.Any) -> FollowedProcess:
        calls.append({'command': command, **kwargs})
        return process

    monkeypatch.setattr(subprocess, 'Popen', start)
    return calls


def test_progress_is_read_on_windows(
    windows: list[list[str]], monkeypatch: pytest.MonkeyPatch
):
    process = FollowedProcess(
        '{}',
        [
            'checked rule 1/2 (line/must-not-have-dangles): 3 errors\r\n',
            'something else\r\n',
            'checked rule 2/2 (line/must-not-intersect): 1 error\r\n',
        ],
    )
    calls = started_as(monkeypatch, process)
    told: list[tuple[int, int, int]] = []
    take = commands.follower(lambda *done: told.append(done))
    completed = runner.run(['tc.exe'], QgsProcessingFeedback(), lines=take)
    assert told == [(0, 2, 3), (1, 2, 1)]
    assert completed == runner.Completed(0, '{}', 'something else\r\n')
    (call,) = calls
    assert call['creationflags'] == CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
    assert call['start_new_session'] is False
    assert call['stdin'] == subprocess.DEVNULL
    assert not call.get('shell')
    assert (call['encoding'], call['errors']) == ('utf-8', 'replace')
    assert process.stdout.closed
    assert process.stderr.closed
    assert windows == []


def test_canceling_on_windows_ends_the_tree_of_the_followed_tool(
    windows: list[list[str]], monkeypatch: pytest.MonkeyPatch
):
    process = FollowedProcess(ends=False)
    started_as(monkeypatch, process)
    feedback = QgsProcessingFeedback()
    feedback.cancel()
    with pytest.raises(RunError, match='Canceled'):
        runner.run(['tc.exe'], feedback, lines=lambda line: True)
    assert windows == [['taskkill', '/T', '/F', '/PID', '4242']]
    assert process.killed
