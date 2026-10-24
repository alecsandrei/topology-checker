from __future__ import annotations

import collections.abc as c
import contextlib
import dataclasses
import json
import os
import queue
import signal

# The plugin runs the topology-checker executable the user installed
import subprocess  # nosec B404
import sys
import threading
import time
import typing as t

from ..exceptions import RunError
from ..utils import not_none, tr

if t.TYPE_CHECKING:
    from qgis.core import QgsProcessingFeedback

POLL_SECONDS = 0.2
# How long to wait for what was killed to be gone
STOP_SECONDS = 5


@dataclasses.dataclass(frozen=True)
class Completed:
    exit_code: int
    stdout: str
    stderr: str


# A line of what the tool says, which says whether it took the line
Lines = c.Callable[[str], bool]


def run(
    command: list[str],
    feedback: QgsProcessingFeedback | None = None,
    timeout: float | None = None,
    lines: Lines | None = None,
) -> Completed:
    """Runs the command without a shell, stopping it if the feedback cancels
    or the timeout in seconds passes.

    The lines, if given, get each line of the error output as soon as the
    tool says it; those they take are not in the output that is returned.
    """
    flags = 0
    if sys.platform == 'win32':
        # Its own group, so that the tree of processes can be stopped
        flags = (
            subprocess.CREATE_NO_WINDOW | subprocess.CREATE_NEW_PROCESS_GROUP
        )
    try:
        # The command is a list of arguments, which no shell sees
        process = subprocess.Popen(  # nosec B603
            command,
            # A program that waits for input would wait for ever
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding='utf-8',
            errors='replace',
            creationflags=flags,
            # Its own group, so that stopping it stops what it started
            start_new_session=sys.platform != 'win32',
        )
    except OSError as error:
        raise RunError(
            tr('Could not run {program}: {error}').format(
                program=command[0], error=error
            )
        ) from error
    started = time.monotonic()

    def check(stopping: c.Callable[[], None]) -> None:
        if feedback is not None and feedback.isCanceled():
            stopping()
            raise RunError(tr('Canceled')) from None
        if timeout is not None and time.monotonic() - started > timeout:
            stopping()
            raise RunError(
                tr('{program} did not answer within {seconds:g} s').format(
                    program=command[0], seconds=timeout
                )
            ) from None

    if lines is not None:
        return follow(process, lines, check)
    while True:
        try:
            stdout, stderr = process.communicate(timeout=POLL_SECONDS)
            break
        except subprocess.TimeoutExpired:
            check(lambda: stop(process))
    return Completed(process.returncode, stdout, stderr)


def follow(
    process: subprocess.Popen[str],
    lines: Lines,
    check: c.Callable[[c.Callable[[], None]], None],
) -> Completed:
    """Waits for the process, giving the lines each line of its error output
    as it comes. A thread reads each pipe, so that neither fills up while
    the other is read."""
    found: queue.Queue[str | None] = queue.Queue()
    out: list[str] = []

    def read_out() -> None:
        try:
            out.append(not_none(process.stdout).read())
        except (OSError, ValueError):
            pass
        finally:
            found.put(None)

    def read_err() -> None:
        try:
            for line in not_none(process.stderr):
                found.put(line)
        except (OSError, ValueError):
            pass
        finally:
            found.put(None)

    readers = [
        threading.Thread(target=read, name='topology-checker-pipe')
        for read in (read_out, read_err)
    ]
    for reader in readers:
        reader.daemon = True
        reader.start()
    kept: list[str] = []
    ends = 0
    while ends < len(readers) or process.poll() is None:
        try:
            line = found.get(timeout=POLL_SECONDS)
        except queue.Empty:
            line = ''
        if line is None:
            ends += 1
        elif line and not lines(line.rstrip('\r\n')):
            kept.append(line)
        check(lambda: stop_followed(process, readers))
    for pipe in (process.stdout, process.stderr):
        if pipe is not None:
            with contextlib.suppress(OSError, ValueError):
                pipe.close()
    return Completed(process.returncode, ''.join(out), ''.join(kept))


def execute(
    command: list[str],
    feedback: QgsProcessingFeedback,
    lines: Lines | None = None,
) -> Completed:
    """Runs the command, showing it and what the tool says in the log. With
    lines, what the tool says is shown as it comes, but for the lines that
    they take."""
    feedback.pushCommandInfo(' '.join(command))
    if lines is None:
        completed = run(command, feedback)
        for line in completed.stderr.splitlines():
            feedback.pushInfo(line)
        return completed

    def told(line: str) -> bool:
        if lines(line):
            return True
        feedback.pushInfo(line)
        return False

    return run(command, feedback, lines=told)


def error_message(completed: Completed) -> str:
    """The message of a failed run, from the JSON error or the plain text."""
    text = completed.stderr.strip() or completed.stdout.strip()
    for line in reversed(text.splitlines()):
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if isinstance(value, dict) and 'error' in value:
            return str(t.cast('dict[str, t.Any]', value)['error'])
    return text or tr('Exited with code {code}').format(
        code=completed.exit_code
    )


def stop(process: subprocess.Popen[str]) -> None:
    """Kills the process and the processes it started, then reaps it.

    A tool behind a launcher script (a .cmd file on Windows) has the script as
    its process: the tool has to be stopped with it, as it would keep the pipes
    open and the output would never end.
    """
    kill(process)
    try:
        process.communicate(timeout=STOP_SECONDS)
    except subprocess.TimeoutExpired:
        # Something that outlived the tree holds the pipes: let go of them
        for pipe in (process.stdout, process.stderr):
            if pipe is not None:
                with contextlib.suppress(OSError, ValueError):
                    pipe.close()
        with contextlib.suppress(subprocess.TimeoutExpired):
            process.wait(timeout=STOP_SECONDS)


def stop_followed(
    process: subprocess.Popen[str], readers: list[threading.Thread]
) -> None:
    """Kills the process and what it started, whose pipes the readers
    read until they are closed, then reaps it."""
    kill(process)
    ends = time.monotonic() + STOP_SECONDS
    for reader in readers:
        reader.join(max(0.0, ends - time.monotonic()))
    with contextlib.suppress(subprocess.TimeoutExpired):
        process.wait(timeout=max(0.0, ends - time.monotonic()))


def kill(process: subprocess.Popen[str]) -> None:
    """Kills the process and the processes it started."""
    if sys.platform == 'win32':
        # The whole tree, which kill() does not reach
        with contextlib.suppress(OSError):
            subprocess.run(  # nosec B603 B607
                ['taskkill', '/T', '/F', '/PID', str(process.pid)],
                capture_output=True,
                creationflags=subprocess.CREATE_NO_WINDOW,
                timeout=STOP_SECONDS,
                check=False,
            )
        with contextlib.suppress(OSError):
            process.kill()
    else:
        with contextlib.suppress(ProcessLookupError):
            os.killpg(process.pid, signal.SIGKILL)
