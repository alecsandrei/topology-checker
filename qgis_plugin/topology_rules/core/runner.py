from __future__ import annotations

import collections
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
# How long a followed tool has to end, and to let go of its pipes, once it
# has told all or has ended
GRACE_SECONDS = 2.0
# How much of what a followed tool says, and the plugin does not take, is
# kept: the end of it
KEPT_CHARS = 1_000_000
# How much of the error output of a tool the log shows: the start as it
# comes, and the end once the tool is done, and how much of each line
LOGGED_CHARS = 50_000
LINE_CHARS = 2_000


@dataclasses.dataclass(frozen=True)
class Completed:
    exit_code: int
    stdout: str
    stderr: str
    # Whether the tool was ended, as it did not end once it had told all
    ended: bool = False


# A line of what the tool says, with its end, which says whether it took the
# line. The end stays on, as taking it off copies the line, and the line of a
# rule with a million errors takes hundreds of megabytes
Lines = c.Callable[[str], bool]


class Tail:
    """The last lines of what a tool says, up to a number of characters, and
    how many lines were left out before them. The last line is kept, however
    long."""

    def __init__(self, size: int) -> None:
        self.size = size
        self.lines: collections.deque[str] = collections.deque()
        self.chars = 0
        self.left_out = 0

    def add(self, line: str) -> None:
        self.lines.append(line)
        self.chars += len(line)
        while self.chars > self.size and len(self.lines) > 1:
            self.chars -= len(self.lines.popleft())
            self.left_out += 1

    def text(self) -> str:
        return ''.join(self.lines)


class Log:
    """Shows lines of the error output of a tool in the log, each cut short:
    the first as they come, and the last once the tool is done. Those in
    between are counted."""

    def __init__(self, feedback: QgsProcessingFeedback) -> None:
        self.feedback = feedback
        self.room = LOGGED_CHARS
        self.tail = Tail(LOGGED_CHARS)

    def add(self, line: str) -> None:
        if len(line) > LINE_CHARS:
            line = line[:LINE_CHARS] + '…'
        if self.room > 0:
            self.room -= len(line)
            self.feedback.pushInfo(line)
        else:
            self.tail.add(line)

    def end(self) -> None:
        if self.tail.left_out:
            self.feedback.pushInfo(
                tr('… {count} lines left out').format(count=self.tail.left_out)
            )
        for line in self.tail.lines:
            self.feedback.pushInfo(line)
        self.tail = Tail(LOGGED_CHARS)


def run(
    command: list[str],
    feedback: QgsProcessingFeedback | None = None,
    timeout: float | None = None,
    lines: Lines | None = None,
    out: Lines | None = None,
    done: c.Callable[[], bool] | None = None,
) -> Completed:
    """Runs the command without a shell, stopping it if the feedback cancels
    or the timeout in seconds passes.

    The lines, if given, get each line of the error output, with its end, as
    soon as the tool says it, and `out` each line of the output; those they
    take are not in what is returned, which has the end of the others. Once
    `done` says that the tool has told all, it has a moment to end, as it
    has once it ends to let go of its pipes; then it is ended.
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

    def failure() -> RunError | None:
        """Why the run stops early, if it does: it is canceled, or out of
        time."""
        if feedback is not None and feedback.isCanceled():
            return RunError(tr('Canceled'))
        if timeout is not None and time.monotonic() - started > timeout:
            return RunError(
                tr('{program} did not answer within {seconds:g} s').format(
                    program=command[0], seconds=timeout
                )
            )
        return None

    if lines is not None or out is not None:
        followed = follow(process, (out, lines), failure, done)
        if isinstance(followed, RunError):
            # Raised here, where no line of the tool was held: the error
            # keeps the frames of its traceback, and Processing keeps it
            raise followed
        return followed
    while True:
        with contextlib.suppress(subprocess.TimeoutExpired):
            stdout, stderr = process.communicate(timeout=POLL_SECONDS)
            break
        if (error := failure()) is not None:
            stop(process)
            raise error
    return Completed(process.returncode, stdout, stderr)


def follow(
    process: subprocess.Popen[str],
    takers: tuple[Lines | None, Lines | None],
    failure: c.Callable[[], RunError | None],
    done: c.Callable[[], bool] | None = None,
) -> Completed | RunError:
    """Waits for the process, giving the takers each line of its output and
    of its error output as it comes. A thread reads each pipe, so that
    neither fills up while the other is read.

    A process that has told all, or has ended, and has not ended and let go
    of its pipes a moment later, is ended, with what it started: the lines
    read by then are taken. A failure, such as a cancel, ends the wait at
    once, and is returned. However the wait ends, what is left of the
    process is stopped and reaped.
    """
    pipes = (not_none(process.stdout), not_none(process.stderr))
    found: queue.Queue[tuple[int, str | None]] = queue.Queue()

    def read(source: int) -> None:
        try:
            for line in pipes[source]:
                found.put((source, line))
                # Not held while the next line is waited for
                del line
        except (OSError, ValueError):
            pass
        finally:
            found.put((source, None))

    readers = [
        threading.Thread(
            target=read,
            args=(source,),
            name='topology-checker-pipe',
            daemon=True,
        )
        for source in range(len(pipes))
    ]
    for reader in readers:
        reader.start()
    kept = (Tail(KEPT_CHARS), Tail(KEPT_CHARS))
    ends = 0
    stopped = False
    ended = False
    failed: RunError | None = None
    # When the process has had its moment to end
    deadline: float | None = None

    def stopping() -> None:
        nonlocal stopped
        stopped = True
        stop_followed(process, readers)

    try:
        while ends < len(readers) or process.poll() is None:
            try:
                source, line = found.get(timeout=POLL_SECONDS)
            except queue.Empty:
                if stopped:
                    # The lines that were read before it ended are taken
                    break
                source, line = 0, ''
            if line is None:
                ends += 1
            elif line:
                take = takers[source]
                if take is None or not take(line):
                    kept[source].add(line)
            failed = failure()
            if failed is not None:
                stopping()
                break
            if stopped:
                continue
            if deadline is None:
                if process.poll() is not None or (done is not None and done()):
                    deadline = time.monotonic() + GRACE_SECONDS
            elif time.monotonic() > deadline and (
                process.poll() is None or any(r.is_alive() for r in readers)
            ):
                ended = process.poll() is None
                stopping()
    finally:
        if not stopped and (
            process.poll() is None or any(r.is_alive() for r in readers)
        ):
            stop_followed(process, readers)
        for reader, pipe in zip(readers, pipes, strict=True):
            # A pipe that is still read is held by what outlived the tree,
            # and closing it would wait for the read
            if not reader.is_alive():
                with contextlib.suppress(OSError, ValueError):
                    pipe.close()
    if failed is not None:
        return failed
    # One that could not be killed has no code
    code = -1 if process.returncode is None else process.returncode
    return Completed(code, kept[0].text(), kept[1].text(), ended)


def execute(
    command: list[str],
    feedback: QgsProcessingFeedback,
    lines: Lines | None = None,
    out: Lines | None = None,
    done: c.Callable[[], bool] | None = None,
) -> Completed:
    """Runs the command, showing it and what the tool says in the log, up to
    a size. With lines or out, what the tool says on its error output is
    shown as it comes, but for the lines that they take, and `out` gets
    each line of the output."""
    feedback.pushCommandInfo(' '.join(command))
    log = Log(feedback)
    if lines is None and out is None:
        completed = run(command, feedback)
        for line in completed.stderr.splitlines():
            log.add(line)
        log.end()
        return completed

    def told(line: str) -> bool:
        if lines is not None and lines(line):
            return True
        log.add(line.rstrip('\r\n'))
        return False

    try:
        return run(command, feedback, lines=told, out=out, done=done)
    finally:
        log.end()


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
        # A group whose last process has ended but is not reaped yet is not
        # there to be killed, which macOS tells as a lack of permission
        with contextlib.suppress(ProcessLookupError, PermissionError):
            os.killpg(process.pid, signal.SIGKILL)
