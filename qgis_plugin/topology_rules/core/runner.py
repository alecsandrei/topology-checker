from __future__ import annotations

import contextlib
import dataclasses
import json
import os
import signal

# The plugin runs the topology-checker executable the user installed
import subprocess  # nosec B404
import sys
import time
import typing as t

from ..exceptions import RunError
from ..utils import tr

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


def run(
    command: list[str],
    feedback: QgsProcessingFeedback | None = None,
    timeout: float | None = None,
) -> Completed:
    """Runs the command without a shell, stopping it if the feedback cancels
    or the timeout in seconds passes."""
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
    while True:
        try:
            stdout, stderr = process.communicate(timeout=POLL_SECONDS)
            break
        except subprocess.TimeoutExpired:
            if feedback is not None and feedback.isCanceled():
                stop(process)
                raise RunError(tr('Canceled')) from None
            if timeout is not None and time.monotonic() - started > timeout:
                stop(process)
                raise RunError(
                    tr('{program} did not answer within {seconds:g} s').format(
                        program=command[0], seconds=timeout
                    )
                ) from None
    return Completed(process.returncode, stdout, stderr)


def execute(command: list[str], feedback: QgsProcessingFeedback) -> Completed:
    """Runs the command, showing it and what the tool says in the log."""
    feedback.pushCommandInfo(' '.join(command))
    completed = run(command, feedback)
    for line in completed.stderr.splitlines():
        feedback.pushInfo(line)
    return completed


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
