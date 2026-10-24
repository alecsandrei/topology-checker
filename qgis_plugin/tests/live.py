"""Runs that a test lets go on one rule at a time, for the tests of what a
run tells while it goes.

With the Python package, each report of `Topology.iter_reports` waits at a
gate until the test lets it through. The executable is a script that runs
the real tool, then says each line of `--progress` when the test lets it.
Nothing waits for a time: what a test waits for is told by the run."""

from __future__ import annotations

import collections.abc as c
import dataclasses
import textwrap
import threading
import typing as t
from pathlib import Path

from qgis.core import (
    QgsGeometry,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
)

from topology_rules.core import package
from topology_rules.core.checks import Job, Outcome, run_jobs
from topology_rules.core.commands import WITHOUT_SUMMARY, validate_command
from topology_rules.core.engine import Tool
from topology_rules.core.package import Errors, attributes, from_wkb, members
from topology_rules.core.progress import Checked, Progress, Stage
from topology_rules.core.runner import Completed, execute
from topology_rules.enums import ExitCode
from topology_rules.exceptions import RunError
from topology_rules.utils import not_none, tr

from .helpers import hand_over, real_executable, write_tool

if t.TYPE_CHECKING:
    import topology_checker as tc

# How long a held run waits for its test before it gives up, which only a
# test that is stuck reaches
HOLD_SECONDS = 120


class Gate:
    """How far a held run of the package may go: how many reports it may
    give, and how many it has at hand."""

    def __init__(self) -> None:
        self.condition = threading.Condition()
        self.allowed = 0
        self.reached = 0
        self.given = 0
        self.closed = 0
        # The report before which the package raises, if any
        self.fail_at: int | None = None
        self.failure = 'the package failed'

    def let(self, count: int) -> None:
        """Lets the run give its first `count` reports."""
        with self.condition:
            self.allowed = max(self.allowed, count)
            self.condition.notify_all()

    def let_all(self) -> None:
        self.let(1 << 30)

    def hold(self, index: int) -> None:
        with self.condition:
            self.reached = max(self.reached, index + 1)
            self.condition.notify_all()
            if not self.condition.wait_for(
                lambda: self.allowed > index, HOLD_SECONDS
            ):
                raise AssertionError('the test did not let the run go on')

    def gave(self, count: int) -> None:
        with self.condition:
            self.given = count
            self.condition.notify_all()

    def ended(self) -> None:
        with self.condition:
            self.closed += 1
            self.condition.notify_all()


class HeldStream:
    """A stream of reports that gives each one when its gate lets it."""

    def __init__(self, stream: tc.ReportStream, gate: Gate) -> None:
        self.stream = stream
        self.gate = gate
        self.count = 0

    def __enter__(self) -> HeldStream:
        self.stream.__enter__()
        return self

    def __exit__(self, *details: t.Any) -> None:
        try:
            self.stream.__exit__(*details)
        finally:
            self.gate.ended()

    def __iter__(self) -> HeldStream:
        return self

    def __next__(self) -> tc.RuleReport:
        index = self.count
        if self.gate.fail_at == index:
            self.gate.hold(index)
            raise ValueError(self.gate.failure)
        report = next(self.stream)
        self.gate.hold(index)
        self.count += 1
        self.gate.gave(self.count)
        return report

    @property
    def summary(self) -> tc.ReportSummary | None:
        return self.stream.summary

    def close(self) -> None:
        self.stream.close()


class Gates:
    """The gate of each stream of reports, in the order the runs ask for
    them. A test can set up the gate of a run before the run starts."""

    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.made: list[Gate] = []
        self.started = 0

    def __getitem__(self, index: int) -> Gate:
        with self.lock:
            while len(self.made) <= index:
                self.made.append(Gate())
            return self.made[index]

    def next(self) -> Gate:
        with self.lock:
            index = self.started
            self.started += 1
        return self[index]


def hold_reports(monkeypatch: t.Any) -> Gates:
    """Holds each report of the package at a gate of the test, one gate for
    each run."""
    import topology_checker as tc

    gates = Gates()
    real = tc.Topology.iter_reports

    def iter_reports(self: tc.Topology) -> HeldStream:
        return HeldStream(real(self), gates.next())

    monkeypatch.setattr(tc.Topology, 'iter_reports', iter_reports)
    return gates


@dataclasses.dataclass
class Hold:
    """A step of a run that waits for the test, while it is armed."""

    armed: bool = True
    reached: threading.Event = dataclasses.field(
        default_factory=threading.Event
    )
    go: threading.Event = dataclasses.field(default_factory=threading.Event)

    def hold(self) -> None:
        if not self.armed:
            return
        self.reached.set()
        if not self.go.wait(HOLD_SECONDS):
            raise AssertionError('the test did not let the run go on')


def hold_calls(
    monkeypatch: t.Any, calls: c.Iterable[tuple[object, str]], armed: bool
) -> Hold:
    """Holds the runs before each of the calls, functions of modules."""
    held_step = Hold(armed)
    for module, name in calls:
        real = getattr(module, name)

        def held(*args: t.Any, real: t.Any = real, **kwargs: t.Any) -> t.Any:
            held_step.hold()
            return real(*args, **kwargs)

        monkeypatch.setattr(module, name, held)
    return held_step


def hold_reading(monkeypatch: t.Any, armed: bool = True) -> Hold:
    """Holds the runs of both engines before they read the layers."""
    from topology_rules.core import checks, package

    return hold_calls(
        monkeypatch, ((package, 'snapshots'), (checks, 'prepare')), armed
    )


def hold_writing(monkeypatch: t.Any, armed: bool = True) -> Hold:
    """Holds the runs of the package before they write the errors."""
    from topology_rules.core import package

    return hold_calls(monkeypatch, ((package, 'write_errors'),), armed)


# Executables


def follow_tool(real: str) -> str:
    """The lines of a script that is the real tool but for a check."""
    return "if 'validate' not in sys.argv:\n" + textwrap.indent(
        hand_over(real), '    '
    )


def replaying_executable(
    directory: Path,
    gates: Path,
    started: Path,
    crash_after: int | None = None,
    killed_after: int | None = None,
    noise: bool = False,
) -> Path:
    """A tool that runs the real tool, and then says each line of
    `--progress` once the test has made the file of the gates folder named
    after its number, or the file 'all'. It writes its process number to the
    started file first.

    It ends once the file 'end' or 'all' is there. It can crash, or kill
    itself, before the line of a rule, and say lines that are not about
    rules, with Windows line ends, bytes that are not UTF-8 and lines that
    are long."""
    real = real_executable()
    assert real is not None, 'set TOPOLOGY_CHECKER'
    gates.mkdir(exist_ok=True)
    source = f"""#!/usr/bin/env python3
import os
import signal
import subprocess
import sys
import time
{follow_tool(real)}
with open({str(started)!r} + '.part', 'w') as file:
    file.write(str(os.getpid()))
os.replace({str(started)!r} + '.part', {str(started)!r})
done = subprocess.run([{real!r}, *sys.argv[1:]], capture_output=True)


def wait(name):
    path = os.path.join({str(gates)!r}, name)
    every = os.path.join({str(gates)!r}, 'all')
    end = time.monotonic() + {HOLD_SECONDS}
    while not os.path.exists(path) and not os.path.exists(every):
        if time.monotonic() > end:
            sys.exit(3)
        time.sleep(0.01)


def say(data):
    sys.stderr.buffer.write(data)
    sys.stderr.buffer.flush()


said = 0
for line in done.stderr.splitlines(keepends=True):
    if line.startswith(b'checked rule '):
        if said == {crash_after!r}:
            wait(str(said))
            say(b"thread 'main' panicked at src/main.rs: the tool broke\\n")
            sys.exit(101)
        if said == {killed_after!r}:
            wait(str(said))
            os.kill(os.getpid(), signal.SIGKILL)
        if {noise!r}:
            say(b'reading roads\\r\\n')
            say(b'caf\\xe9 \\xff\\xfe not utf-8\\n')
            say(b'x' * 100000 + b'\\n')
            say(b'checked rule 0/0 (nothing): 1 error\\n')
        wait(str(said))
        said += 1
    say(line)
if {noise!r}:
    say(b'done\\r\\n')
# As the tool, which writes its file after the last rule
wait('end')
sys.stdout.buffer.write(done.stdout)
sys.stdout.buffer.flush()
sys.exit(done.returncode)
"""
    return write_tool(directory / 'topology-checker', source)


def let_lines(gates: Path, count: int) -> None:
    """Lets a replaying tool say its first `count` lines of progress."""
    for number in range(count):
        (gates / str(number)).touch()


def let_all_lines(gates: Path) -> None:
    (gates / 'all').touch()


def let_end(gates: Path) -> None:
    """Lets a replaying tool end, once it has said its lines."""
    (gates / 'end').touch()


def close_gates(gates: Path) -> None:
    """Holds the next run of a replaying tool from the start."""
    for gate in gates.iterdir():
        gate.unlink()


def refusing_executable(
    directory: Path, name: str = 'topology-checker'
) -> Path:
    """A tool from before --progress: it refuses the flag as the parser of
    its arguments does, and is the real tool otherwise."""
    real = real_executable()
    assert real is not None, 'set TOPOLOGY_CHECKER'
    source = f"""#!/usr/bin/env python3
import os
import sys
if '--progress' in sys.argv:
    print("error: unexpected argument '--progress' found", file=sys.stderr)
    print('', file=sys.stderr)
    print('Usage: topology-checker validate <CONFIG>', file=sys.stderr)
    print('', file=sys.stderr)
    print("For more information, try '--help'.", file=sys.stderr)
    sys.exit(2)
{hand_over(real)}"""
    return write_tool(directory / name, source)


def forwarding_executable(
    directory: Path, log: Path, name: str = 'topology-checker'
) -> Path:
    """The real tool, behind a script that writes down each of its
    commands."""
    real = real_executable()
    assert real is not None, 'set TOPOLOGY_CHECKER'
    source = f"""#!/usr/bin/env python3
import os
import sys
with open({str(log)!r}, 'a') as log:
    log.write(' '.join(sys.argv[1:]) + '\\n')
{hand_over(real)}"""
    return write_tool(directory / name, source)


# What a run tells


Event = tuple[Stage, int | None, int] | Checked


class Recorder(Progress):
    """A feedback that keeps what the run told it, in order."""

    def __init__(self) -> None:
        super().__init__()
        self.lock = threading.Lock()
        self.events: list[Event] = []

    def staged(self, stage: Stage, index: int | None, total: int) -> None:
        with self.lock:
            self.events.append((stage, index, total))

    def checked(self, done: Checked) -> None:
        with self.lock:
            self.events.append(done)

    def checks(self) -> list[Checked]:
        return [event for event in self.events if isinstance(event, Checked)]

    def stages(self) -> list[tuple[Stage, int | None, int]]:
        return [
            event for event in self.events if not isinstance(event, Checked)
        ]


@dataclasses.dataclass
class Told:
    outcome: Outcome
    feedback: Recorder
    output: Path


def run_told(
    tool: Tool,
    jobs: list[Job],
    folder: Path,
    tolerance: float | None = None,
    feedback: Recorder | None = None,
) -> Told:
    """Runs the jobs as the algorithm does, and keeps what the run told."""
    folder.mkdir(parents=True)
    work = folder / 'work'
    work.mkdir()
    output = folder / 'errors.gpkg'
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    feedback = feedback or Recorder()
    outcome = run_jobs(
        jobs, tool, tolerance, str(output), work, context, feedback
    )
    return Told(outcome, feedback, output)


def expected_stages(
    total: int, executable: bool
) -> list[tuple[Stage, int | None, int]]:
    """What a run of a number of rules says it does, from start to end: the
    executable does not say which rule it checks until the first is done."""
    first: tuple[Stage, int | None, int] = (
        (Stage.CHECKING, None, total)
        if executable
        else (Stage.CHECKING, 0, total)
    )
    return [
        (Stage.READING, None, 0),
        first,
        *((Stage.CHECKING, index, total) for index in range(1, total)),
        (Stage.WRITING, None, total),
    ]


def wkt_of(geometry: QgsGeometry) -> str:
    return geometry.asWkt(6)


def found_rows(checks: c.Iterable[Checked]) -> dict[str, list[list[object]]]:
    """The errors that the run told of, by family, as the layers of errors
    hold them: the geometry, then the columns."""
    rows: dict[str, list[list[object]]] = {}
    for done in checks:
        for found in done.errors or ():
            rows.setdefault(found.family, []).append(
                [wkt_of(found.geometry), *found.values]
            )
    return rows


def project_layers(*layers: t.Any) -> None:
    for layer in layers:
        QgsProject.instance().addMapLayer(layer)


# The runs as they were before the engines told of each rule


def validate_untold(
    topology: t.Any,
    registered: c.Iterable[package.Registered],
    jobs: t.Sequence[Job],
    layers: t.Mapping[str, str],
    feedback: QgsProcessingFeedback,
) -> Errors:
    """How the package was run: one stream of all the errors."""
    import topology_checker as tc

    names = {entry.layer: entry.name for entry in registered}
    rows: dict[str, list[tuple[QgsGeometry, list[object]]]] = {}
    if feedback.isCanceled():
        raise RunError(tr('Canceled'))
    try:
        with topology.iter_validate() as stream:
            for index, violation in enumerate(stream):
                if feedback.isCanceled():
                    raise RunError(tr('Canceled'))
                values = attributes(index, violation, names)
                for member in members(from_wkb(violation.wkb)):
                    family = package.wkb_kind(member.wkbType())
                    rows.setdefault(family, []).append((member, values))
            summary = not_none(stream.summary)
    except (ValueError, tc.PrecisionError) as error:
        raise RunError(str(error)) from error
    return Errors(
        rows,
        [rule.error_count for rule in summary.rules],
        list(summary.warnings),
    )


def run_validate_untold(
    executable: str,
    config: str,
    tolerance: float | None,
    output: str,
    feedback: QgsProcessingFeedback,
    done: object = None,
) -> Completed:
    """How the executable was run: without --progress, its error output
    read at the end."""
    if executable not in WITHOUT_SUMMARY:
        completed = execute(
            validate_command(executable, config, tolerance, output, True),
            feedback,
        )
        known = completed.exit_code != ExitCode.ERROR.value or (
            '--summary' not in completed.stderr + completed.stdout
        )
        if known:
            return completed
        WITHOUT_SUMMARY.add(executable)
    return execute(
        validate_command(executable, config, tolerance, output), feedback
    )
