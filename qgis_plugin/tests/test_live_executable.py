"""The executable with --stream: what it says is read as it says it, from
both pipes at once, whatever bytes and line ends it uses, and a canceled
run ends it at once. The report of each rule is read as soon as the tool
says it; what is not of the report, or is cut off, is left out. A tool that
does not know the flag says its report at the end, and is asked again once
it is replaced."""

from __future__ import annotations

import contextlib
import errno
import json
import os
import signal
import subprocess
import sys
import threading
import time
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsProcessingFeedback

from topology_rules.core import commands, runner
from topology_rules.core.checks import Job
from topology_rules.exceptions import RunError
from topology_rules.utils import not_none

from .helpers import process_alive, wait_for, write_tool
from .live import (
    Recorder,
    Told,
    expected_stages,
    found_rows,
    let_all_lines,
    let_end,
    let_lines,
    refusing_executable,
    replaying_executable,
    run_told,
)
from .test_live_engines import file_rows, jobs_of, lines

not_on_windows = pytest.mark.skipif(
    sys.platform == 'win32', reason='a test of the sessions of POSIX'
)

# The start of each tool here: it says bytes on either pipe, and waits for
# the files that the test makes
PRELUDE = """#!/usr/bin/env python3
import os
import subprocess
import sys
import time


def say(data, stream=sys.stderr):
    stream.buffer.write(data)
    stream.buffer.flush()


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
    """No tool is known to refuse --stream before the test."""
    monkeypatch.setattr(commands, 'WITHOUT_STREAM', set())


# Reading what the tool says


@pytest.mark.parametrize('pipe', ['stdout', 'stderr'])
def test_each_line_is_read_as_soon_as_the_tool_says_it(
    tmp_path: Path, pipe: str
):
    """The tool says its next line only once the test has read the last:
    a run that read the lines at the end would wait for ever."""
    other = 'stderr' if pipe == 'stdout' else 'stdout'
    gates = tmp_path / 'gates'
    gates.mkdir()
    command = tool(
        tmp_path / 'tool',
        f"""
for number in range(3):
    say(f'line {{number}}\\n'.encode(), sys.{pipe})
    wait(os.path.join({str(gates)!r}, str(number)))
say(b'other\\n', sys.{other})
""",
    )
    seen: list[str] = []

    def take(line: str) -> bool:
        seen.append(line)
        (gates / str(len(seen) - 1)).touch()
        return True

    taker = {'out': take} if pipe == 'stdout' else {'lines': take}
    completed = runner.run([command], QgsProcessingFeedback(), **taker)
    assert completed.exit_code == 0
    assert seen == ['line 0', 'line 1', 'line 2']
    assert getattr(completed, other) == 'other\n'
    assert getattr(completed, pipe) == ''


def test_both_pipes_are_read_while_the_tool_fills_them(tmp_path: Path):
    """Far more than a pipe holds goes to each, in turns."""
    command = tool(
        tmp_path / 'tool',
        """
for number in range(2000):
    sys.stdout.write(f'o{number:04d}' + 'o' * 1000 + '\\n')
    say(f'e{number}\\n'.encode())
sys.stdout.flush()
""",
    )
    taken: list[str] = []
    out: list[str] = []

    def take(line: str) -> bool:
        if int(line[1:]) % 2 == 0:
            taken.append(line)
            return True
        return False

    def take_out(line: str) -> bool:
        if int(line[1:5]) % 2 == 1:
            out.append(line)
            return True
        return False

    # A run that read one pipe only would stop here
    completed = runner.run([command], timeout=60, lines=take, out=take_out)
    assert completed.exit_code == 0
    assert out == [f'o{n:04d}' + 'o' * 1000 for n in range(1, 2000, 2)]
    assert completed.stdout == ''.join(
        f'o{n:04d}' + 'o' * 1000 + '\n' for n in range(0, 2000, 2)
    )
    assert taken == [f'e{n}' for n in range(0, 2000, 2)]
    assert completed.stderr == ''.join(f'e{n}\n' for n in range(1, 2000, 2))


def test_the_report_is_read_whatever_else_the_tool_says(tmp_path: Path):
    """Windows line ends, bytes that are not UTF-8, a long line, a line in
    two writes and a last line with no end."""
    command = tool(
        tmp_path / 'tool',
        """
out = sys.stdout
say(b'reading roads\\r\\n', out)
say(b'{"rule": 1}\\r\\n', out)
say(b'caf\\xe9 \\xff\\xfe\\n', out)
say(b'"' + b'x' * 1000000 + b'"\\n', out)
say(b'{"rule": ', out)
say(b'2}\\n', out)
say(b'not json\\n', out)
say(b'{"rule": 3}', out)
""",
    )
    taken: list[object] = []

    def take(line: str) -> bool:
        try:
            value = json.loads(line)
        except ValueError:
            return False
        if not isinstance(value, dict):
            return False
        taken.append(value)
        return True

    completed = runner.run([command], QgsProcessingFeedback(), out=take)
    assert completed.exit_code == 0
    assert taken == [{'rule': 1}, {'rule': 2}, {'rule': 3}]
    assert completed.stdout == (
        'reading roads\n'
        + b'caf\xe9 \xff\xfe\n'.decode('utf-8', 'replace')
        + '"'
        + 'x' * 1000000
        + '"\n'
        + 'not json\n'
    )


def test_a_reader_that_fails_ends_the_tool(tmp_path: Path):
    """What raises while a line is read ends the tool and what it started,
    which are reaped, and goes on up."""
    pids = tmp_path / 'pids'
    command = tool(
        tmp_path / 'tool',
        f"""
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(600)'])
tell({str(pids)!r}, f'{{os.getpid()}} {{child.pid}}')
say(b'first\\n', sys.stdout)
time.sleep(600)
""",
    )

    class Broken(Exception):
        pass

    def take(line: str) -> bool:
        raise Broken(line)

    with pytest.raises(Broken, match='first'):
        runner.run([command], QgsProcessingFeedback(), out=take)
    started, child = numbers(pids)
    # A process that is not reaped is still there
    wait_for(lambda: not process_alive(started), 30)
    wait_for(lambda: not process_alive(child), 30)


@not_on_windows
@pytest.mark.parametrize('error', [PermissionError, ProcessLookupError])
def test_a_group_that_is_gone_is_not_an_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, error: type[OSError]
):
    """macOS tells of a group whose processes ended, and are not reaped
    yet, as a lack of permission."""
    process = subprocess.Popen(
        [tool(tmp_path / 'tool', '')],
        stdout=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    # Its output ends as it ends, and it waits to be reaped
    assert not_none(process.stdout).read() == ''
    runner.kill(process)
    sent: list[int] = []

    def killpg(group: int, number: int) -> None:
        sent.append(group)
        raise error(errno.EPERM, 'gone')

    monkeypatch.setattr(os, 'killpg', killpg)
    runner.stop(process)
    assert sent == [process.pid]
    assert process.returncode == 0


# Canceling


def test_canceling_ends_the_tool_and_what_it_started_at_once(tmp_path: Path):
    pids = tmp_path / 'pids'
    command = tool(
        tmp_path / 'tool',
        f"""
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(600)'])
tell({str(pids)!r}, f'{{os.getpid()}} {{child.pid}}')
say(b'ready\\n', sys.stdout)
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
        runner.run([command], feedback, out=take)
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
say(b'ready\\n', sys.stdout)
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
            runner.run([command], feedback, out=take)
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
    command = tool(
        tmp_path / 'tool',
        "say(b'only line\\n')\nsay(b'out\\n', sys.stdout)\nsys.exit(4)\n",
    )
    completed = runner.run(
        [command],
        QgsProcessingFeedback(),
        lines=lambda line: False,
        out=lambda line: False,
    )
    assert completed == runner.Completed(4, 'out\n', 'only line\n')


# The report as the tool says it


def replaying(tmp_path: Path, **how: t.Any) -> tuple[str, Path]:
    """A replaying tool, and the folder of its gates."""
    folder = tmp_path / 'tool'
    folder.mkdir()
    gates = tmp_path / 'gates'
    path = replaying_executable(folder, gates, tmp_path / 'started', **how)
    return str(path), gates


class Running:
    """A run in a thread of its own, as in a task."""

    def __init__(self, tool: str, jobs: list[Job], folder: Path) -> None:
        self.recorder = Recorder()
        self.result: list[Told | BaseException] = []
        self.thread = threading.Thread(
            target=self.run, args=(tool, jobs, folder), daemon=True
        )
        self.thread.start()

    def run(self, tool: str, jobs: list[Job], folder: Path) -> None:
        try:
            self.result.append(
                run_told(tool, jobs, folder, feedback=self.recorder)
            )
        except BaseException as error:
            self.result.append(error)

    def told(self) -> Told:
        self.thread.join(120)
        (result,) = self.result
        if isinstance(result, BaseException):
            raise result
        return result


def test_each_rule_is_read_as_soon_as_the_tool_says_it(
    executable: str, tmp_path: Path
):
    """The tool says the report of the next rule only once the test lets
    it, which it does once the run told of the last."""
    path, gates = replaying(tmp_path)
    jobs = jobs_of(path, lines())
    reference = run_told(executable, jobs, tmp_path / 'reference')
    stages = expected_stages(len(jobs))
    running = Running(path, jobs, tmp_path / 'run')
    recorder = running.recorder
    try:
        for done in range(1, len(jobs) + 1):
            let_lines(gates, done)
            wait_for(lambda done=done: len(recorder.checks()) == done)
            # It says which rule is next, or that it writes the errors
            assert recorder.stages()[-1] == stages[done + 1]
            # The next rule is not told before it is let through
            assert len(recorder.checks()) == done
    finally:
        let_all_lines(gates)
        let_end(gates)
    ran = running.told()
    assert ran.outcome.counts == reference.outcome.counts
    assert file_rows(ran.output) == file_rows(reference.output)
    assert found_rows(recorder.checks()) == file_rows(reference.output)
    assert recorder.stages() == expected_stages(len(jobs))


def test_lines_that_are_not_of_the_report_are_left_out(
    executable: str, tmp_path: Path
):
    """What the tool says on its error output is in the log; lines of its
    output that are not of the report, or of no rule of the run, are left
    out, and the rules are told as from a quiet tool."""
    noisy, gates = replaying(tmp_path, noise=True)
    let_all_lines(gates)
    let_end(gates)
    jobs = jobs_of(noisy, lines())
    told = run_told(noisy, jobs, tmp_path / 'run')
    reference = run_told(executable, jobs, tmp_path / 'reference')
    assert told.outcome.counts == reference.outcome.counts
    assert [d.count for d in told.feedback.checks()] == (
        reference.outcome.counts
    )
    assert file_rows(told.output) == file_rows(reference.output)
    assert found_rows(told.feedback.checks()) == file_rows(reference.output)
    assert told.feedback.stages() == expected_stages(len(jobs))
    log = told.feedback.textLog()
    assert 'reading roads' in log
    assert b'caf\xe9 \xff\xfe not utf-8'.decode('utf-8', 'replace') in log
    assert 'x' * 100000 in log
    assert '[' * 1000 not in log
    assert '9' * 4000 not in log
    assert 'line/no-such-rule' not in log


@not_on_windows
@pytest.mark.parametrize('done', [0, 1, 2])
def test_a_tool_killed_halfway_through_a_line_fails(
    executable: str, tmp_path: Path, done: int
):
    """The rules before it are told; the line that is cut off is not."""
    path, gates = replaying(tmp_path, killed_after=done)
    let_all_lines(gates)
    jobs = jobs_of(path, lines())
    reference = run_told(executable, jobs, tmp_path / 'reference')
    recorder = Recorder()
    with pytest.raises(RunError, match=f'Exited with code {-signal.SIGKILL}'):
        run_told(path, jobs, tmp_path / 'run', feedback=recorder)
    counts = [d.count for d in recorder.checks()]
    assert counts == reference.outcome.counts[:done]
    assert not (tmp_path / 'run' / 'errors.gpkg').exists()


def test_a_tool_that_crashes_fails_with_what_it_said(
    executable: str, tmp_path: Path
):
    path, gates = replaying(tmp_path, crash_after=1)
    let_all_lines(gates)
    jobs = jobs_of(path, lines())
    recorder = Recorder()
    with pytest.raises(RunError, match='the tool broke'):
        run_told(path, jobs, tmp_path / 'run', feedback=recorder)
    assert len(recorder.checks()) == 1


@pytest.mark.parametrize('done', [0, 1, 2, 3])
def test_a_canceled_run_keeps_the_rules_that_are_done(
    executable: str, tmp_path: Path, done: int
):
    """The tool is ended at once, and the run ends with the errors of the
    rules that it told of, as a whole run has them; with none it fails. A
    run canceled once the tool said all rules is whole."""
    path, gates = replaying(tmp_path)
    jobs = jobs_of(path, lines())
    reference = run_told(executable, jobs, tmp_path / 'reference')
    whole = file_rows(reference.output)
    running = Running(path, jobs, tmp_path / 'run')
    started = tmp_path / 'started'
    try:
        let_lines(gates, done)
        wait_for(started.exists)
        wait_for(lambda: len(running.recorder.checks()) == done)
        running.recorder.cancel()
        if done == 0:
            with pytest.raises(RunError, match='Canceled'):
                running.told()
            return
        told = running.told()
    finally:
        pid = int(started.read_text()) if started.exists() else None
        let_all_lines(gates)
        let_end(gates)
    assert pid is not None
    wait_for(lambda: not process_alive(pid), 30)
    found = sum(reference.outcome.counts[:done])
    assert told.outcome.counts == reference.outcome.counts[:done]
    assert told.outcome.layers == reference.outcome.layers
    assert file_rows(told.output) == {
        family: kept
        for family, rows in whole.items()
        if (kept := [row for row in rows if t.cast(int, row[1]) < found])
    }
    assert found_rows(running.recorder.checks()) == file_rows(told.output)


# A tool from before --stream


def validate_lines(log: Path) -> list[list[str]]:
    return [
        line.split()
        for line in log.read_text().splitlines()
        if 'validate' in line.split()
    ]


def test_a_tool_without_stream_tells_the_rules_at_the_end(
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
    assert ['--stream' in command for command in asked] == [True, False]
    # It writes no file of its own: the report has the errors
    assert all('-o' not in command for command in asked)
    # The rules are told once it is done, with their errors
    assert [d.index for d in told.feedback.checks()] == list(range(len(jobs)))
    assert found_rows(told.feedback.checks()) == file_rows(told.output)
    assert told.feedback.stages() == expected_stages(
        len(jobs), streams=False, refused=True
    )
    # The next run does not ask with the flag again
    again = run_told(old, jobs, tmp_path / 'again')
    assert ['--stream' in command for command in validate_lines(log)] == [
        True,
        False,
        False,
    ]
    assert again.feedback.stages() == expected_stages(len(jobs), streams=False)
    assert file_rows(again.output) == file_rows(reference.output)


def test_a_tool_that_is_replaced_at_its_path_is_asked_again(
    executable: str, tmp_path: Path
):
    folder = tmp_path / 'tool'
    folder.mkdir()
    path = refusing_executable(folder)
    jobs = jobs_of(str(path), lines())
    run_told(str(path), jobs, tmp_path / 'old')
    assert len(commands.WITHOUT_STREAM) == 1
    # A newer tool at the same path, which knows the flag
    changed = os.stat(path).st_mtime_ns
    gates = tmp_path / 'gates'
    replaced = replaying_executable(folder, gates, tmp_path / 'started')
    assert replaced == path
    later = changed + 10**9
    os.utime(path, ns=(later, later))
    let_all_lines(gates)
    let_end(gates)
    told = run_told(str(path), jobs, tmp_path / 'new')
    assert (tmp_path / 'started').exists()
    assert told.feedback.stages() == expected_stages(len(jobs))
    assert [done.index for done in told.feedback.checks()] == list(
        range(len(jobs))
    )


# Windows, with a patched platform and a process that is not started


CREATE_NO_WINDOW = 0x08000000
CREATE_NEW_PROCESS_GROUP = 0x00000200


class Lines:
    """A pipe that has its lines at once."""

    def __init__(self, lines: list[str]) -> None:
        self.lines = lines
        self.closed = False

    def __iter__(self) -> t.Iterator[str]:
        return iter(self.lines)

    def read(self) -> str:
        return ''.join(self.lines)

    def close(self) -> None:
        self.closed = True


class FollowedProcess:
    """A process whose pipes have their lines at once, which ends when they
    are read, or runs until it is killed."""

    def __init__(
        self,
        stdout: list[str] | None = None,
        stderr: list[str] | None = None,
        ends: bool = True,
    ) -> None:
        self.pid = 4242
        self.returncode: int | None = None
        self.ends = ends
        self.killed = False
        self.stdout = Lines(stdout or [])
        self.stderr = Lines(stderr or [])

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


def test_the_report_is_read_on_windows(
    windows: list[list[str]], monkeypatch: pytest.MonkeyPatch
):
    process = FollowedProcess(
        ['{"rule": 1}\r\n', 'something else\r\n', '{"rule": 2}\r\n'],
        ['reading\r\n'],
    )
    calls = started_as(monkeypatch, process)
    told: list[str] = []

    def take(line: str) -> bool:
        if line.startswith('{'):
            told.append(line)
            return True
        return False

    completed = runner.run(['tc.exe'], QgsProcessingFeedback(), out=take)
    assert told == ['{"rule": 1}', '{"rule": 2}']
    assert completed == runner.Completed(0, 'something else\r\n', 'reading\r\n')
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
        runner.run(['tc.exe'], feedback, out=lambda line: True)
    assert windows == [['taskkill', '/T', '/F', '/PID', '4242']]
    assert process.killed
