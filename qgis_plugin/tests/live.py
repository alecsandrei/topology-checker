"""Runs that a test lets go on one rule at a time, for the tests of what a
run tells while it goes.

With the Python package, each report of `Topology.iter_reports` waits at a
gate until the test lets it through. The executable is a script that runs
the real tool, then says the report of each rule, as `--stream` does, when
the test lets it. Nothing waits for a time: what a test waits for is told by
the run."""

from __future__ import annotations

import collections.abc as c
import dataclasses
import shutil
import textwrap
import threading
import typing as t
from pathlib import Path

from qgis.core import (
    QgsFeature,
    QgsGeometry,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
)

from topology_rules.core import package
from topology_rules.core.checks import Job, Outcome, run_jobs
from topology_rules.core.commands import ERROR_LAYERS, validate_command
from topology_rules.core.engine import Tool
from topology_rules.core.errors import Errors, Gathered, found_in, from_wkb
from topology_rules.core.layers import private_layer
from topology_rules.core.package import attributes
from topology_rules.core.progress import Checked, Found, Progress, Stage
from topology_rules.core.report import IdMap, Report, source_id
from topology_rules.core.runner import Completed, execute
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
    """Holds the runs of both engines before they write the errors."""
    from topology_rules.core import checks, package

    return hold_calls(
        monkeypatch,
        ((package, 'write_errors'), (checks, 'write_errors')),
        armed,
    )


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
    """A tool that runs the real tool, and then says the report of each rule
    on a line of its own, as `--stream` has it, once the test has made the
    file of the gates folder named after its number, or the file 'all'. It
    writes its process number to the started file first. Without --stream
    it is the real tool.

    It says the last line, and ends, once the file 'end' or 'all' is there.
    It can crash before the line of a rule, or kill itself halfway through
    the line, and say lines that are not of the report: with Windows line
    ends, bytes that are not UTF-8, lines that are long or nested deep,
    numbers of thousands of digits, and reports of rules that are not of
    the run."""
    real = real_executable()
    assert real is not None, 'set TOPOLOGY_CHECKER'
    gates.mkdir(exist_ok=True)
    source = f"""#!/usr/bin/env python3
import json
import os
import signal
import subprocess
import sys
import time
{follow_tool(real)}
if '--stream' not in sys.argv:
{textwrap.indent(hand_over(real), '    ')}
with open({str(started)!r} + '.part', 'w') as file:
    file.write(str(os.getpid()))
os.replace({str(started)!r} + '.part', {str(started)!r})
arguments = [argument for argument in sys.argv[1:] if argument != '--stream']
done = subprocess.run([{real!r}, *arguments], capture_output=True)
sys.stderr.buffer.write(done.stderr)
sys.stderr.buffer.flush()
try:
    report = json.loads(done.stdout)
except ValueError:
    sys.stdout.buffer.write(done.stdout)
    sys.exit(done.returncode)


def wait(name):
    path = os.path.join({str(gates)!r}, name)
    every = os.path.join({str(gates)!r}, 'all')
    end = time.monotonic() + {HOLD_SECONDS}
    while not os.path.exists(path) and not os.path.exists(every):
        if time.monotonic() > end:
            sys.exit(3)
        time.sleep(0.01)


def say(data, stream=sys.stdout):
    stream.buffer.write(data)
    stream.buffer.flush()


OTHER = {{
    'spec': {{'rule': 'line/no-such-rule', 'layer': 0, 'other': None}},
    'layer': 'x',
    'other': None,
    'counts': {{'dangle': 1}},
    'errors': [],
}}
for number, rule in enumerate(report.pop('rules')):
    line = json.dumps(rule).encode()
    if number == {crash_after!r}:
        wait(str(number))
        say(b"thread 'main' panicked at main.rs: the tool broke\\n", sys.stderr)
        sys.exit(101)
    if number == {killed_after!r}:
        wait(str(number))
        say(line[: len(line) // 2])
        os.kill(os.getpid(), signal.SIGKILL)
    if {noise!r}:
        say(b'reading roads\\r\\n', sys.stderr)
        say(b'caf\\xe9 \\xff\\xfe not utf-8\\n', sys.stderr)
        say(b'x' * 100000 + b'\\n', sys.stderr)
        say(b'reading roads\\r\\n')
        say(b'caf\\xe9 \\xff\\xfe not utf-8\\n')
        say(b'x' * 100000 + b'\\n')
        say(b'[' * 100000 + b'\\n')
        say(b'9' * 5000 + b'\\n')
        say(b'{{"spec": {{"layer": ' + b'9' * 5000 + b'}}}}\\n')
        say(b'{{"spec": null, "errors": [1]}}\\n')
        say(json.dumps(OTHER).encode() + b'\\n')
        say(b'null\\n')
    wait(str(number))
    say(line + b'\\r\\n' if {noise!r} else line + b'\\n')
if {noise!r}:
    say(b'done\\r\\n', sys.stderr)
# As the tool, which writes its file after the last rule
wait('end')
say(json.dumps(report).encode() + b'\\n')
sys.exit(done.returncode)
"""
    return write_tool(directory / 'topology-checker', source)


def let_lines(gates: Path, count: int) -> None:
    """Lets a replaying tool say the reports of its first `count` rules."""
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
    directory: Path, name: str = 'topology-checker', log: Path | None = None
) -> Path:
    """A tool from before --stream: it refuses the flag as the parser of
    its arguments does, and is the real tool otherwise. With a log it writes
    down each of its commands."""
    real = real_executable()
    assert real is not None, 'set TOPOLOGY_CHECKER'
    logged = (
        ''
        if log is None
        else f"""with open({str(log)!r}, 'a') as log:
    log.write(' '.join(sys.argv[1:]) + '\\n')
"""
    )
    source = f"""#!/usr/bin/env python3
import os
import sys
{logged}if '--stream' in sys.argv:
    print("error: unexpected argument '--stream' found", file=sys.stderr)
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
    total: int, streams: bool = True, refused: bool = False
) -> list[tuple[Stage, int | None, int]]:
    """What a run of a number of rules says it does, from start to end. An
    executable that is known not to stream says which rule it checks only
    once all are done; one that refuses to stream is first taken to."""
    first: list[tuple[Stage, int | None, int]] = (
        [(Stage.CHECKING, 0, total)] if streams or refused else []
    )
    if not streams:
        first.append((Stage.CHECKING, None, total))
    return [
        (Stage.READING, None, 0),
        *first,
        *((Stage.CHECKING, index, total) for index in range(1, total)),
        (Stage.WRITING, None, total),
    ]


def wkt_of(geometry: QgsGeometry) -> tuple[str, str]:
    """The geometry as text, and as WKB to the last bit of each coordinate,
    as the tests of the files read it."""
    return geometry.asWkt(6), bytes(geometry.asWkb()).hex()


def found_rows(checks: c.Iterable[Checked]) -> dict[str, list[list[object]]]:
    """The errors that the run told of, by family, as the layers of errors
    hold them: the geometry, then the columns."""
    rows: dict[str, list[list[object]]] = {}
    for done in checks:
        for found in done.errors:
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
    rows: dict[str, list[Found]] = {}
    if feedback.isCanceled():
        raise RunError(tr('Canceled'))
    try:
        with topology.iter_validate() as stream:
            for index, violation in enumerate(stream):
                if feedback.isCanceled():
                    raise RunError(tr('Canceled'))
                values = attributes(index, violation, names)
                for found in found_in(from_wkb(violation.wkb), values):
                    rows.setdefault(found.family, []).append(found)
            summary = not_none(stream.summary)
    except (ValueError, tc.PrecisionError) as error:
        raise RunError(str(error)) from error
    return Errors(
        rows,
        [rule.error_count for rule in summary.rules],
        list(summary.warnings),
    )


class Untold(Gathered):
    """The errors of the rules, of which nothing is told."""

    def add(self, found: c.Sequence[Found], count: int) -> None:
        for item in found:
            self.rows.setdefault(item.family, []).append(item)
        self.counts.append(count)


def renumber_errors(path: str, ids: IdMap) -> None:
    """Gives the errors of a GeoPackage the ids of the source features."""
    for name in ERROR_LAYERS:
        layer = private_layer(f'{path}|layername={name}', name)
        if not layer.isValid():
            continue
        fields = layer.fields()
        changes: dict[int, dict[int, object]] = {}
        for feature in t.cast('c.Iterable[QgsFeature]', layer.getFeatures()):
            new: dict[int, object] = {}
            for layer_column, id_column in (
                ('origin_layer', 'origin_fid'),
                ('destination_layer', 'destination_fid'),
            ):
                before = feature[id_column]
                after = source_id(ids, feature[layer_column], before)
                if after != before:
                    new[fields.indexOf(id_column)] = after
            if new:
                changes[feature.id()] = new
        if changes:
            assert layer.startEditing()
            for fid, values in changes.items():
                assert layer.changeAttributeValues(fid, values)
            assert layer.commitChanges()


def untold_executable(monkeypatch: t.Any) -> None:
    """How the executable was run: it wrote the errors itself, to the file
    of the output, and the plugin gave them the ids of the source features.
    Nothing was told of a rule until the end."""
    from topology_rules.core import checks

    tables: list[IdMap] = []
    written: list[Path] = []

    class Recorded(Report):
        def __init__(self, gathered: Gathered, ids: IdMap) -> None:
            super().__init__(gathered, ids)
            tables.append(ids)

    def run_validate(
        executable: str,
        config: str,
        tolerance: float | None,
        feedback: QgsProcessingFeedback,
        out: c.Callable[[str], bool],
        older: object = None,
    ) -> Completed:
        own = Path(config).with_name('own.gpkg')
        command = validate_command(executable, config, tolerance, False)
        completed = execute([*command, '-o', str(own), '--overwrite'], feedback)
        written.append(own)
        for line in completed.stdout.splitlines():
            out(line)
        return completed

    def write_errors(path: str, *_: object) -> None:
        shutil.copyfile(written[-1], path)
        renumber_errors(path, tables[-1])

    monkeypatch.setattr(checks, 'Gathered', Untold)
    monkeypatch.setattr(checks, 'Report', Recorded)
    monkeypatch.setattr(checks, 'run_validate', run_validate)
    monkeypatch.setattr(checks, 'write_errors', write_errors)


def untold(monkeypatch: t.Any) -> None:
    """Both engines as they ran before they told of each rule."""
    monkeypatch.setattr(package, 'validate', validate_untold)
    untold_executable(monkeypatch)
