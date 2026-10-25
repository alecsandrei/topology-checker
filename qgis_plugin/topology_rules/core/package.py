"""Runs the checks inside QGIS with the topology-checker Python package.

The package reads the features as they are in the layers, as WKB, with the
ids of the layers, and the errors are written as the executable writes
them. The package is imported by the run that uses it: the module also
loads where it is not installed.
"""

from __future__ import annotations

import collections.abc as c
import contextlib
import dataclasses
import queue
import threading
import typing as t

from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsGeometry,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsVectorFileWriter,
    QgsVectorLayer,
    QgsWkbTypes,
)

from ..exceptions import RunError
from ..utils import not_none, tr
from .binary import Rule, known
from .checks import Job, Outcome
from .commands import unique
from .errors import (
    MULTI,
    Errors,
    ErrorsFile,
    Gathered,
    create_layer,
    found_in,
    from_wkb,
    write_rows,
)
from .inputs import (
    Skipped,
    geometries,
    invalid_reason,
    left_out,
    not_finite,
    own_file,
    reconnect,
    reprojection,
    require_readable,
    stem,
    straight,
)
from .invalid import Invalid, Rejected, parse_failure
from .layers import Dataset, Input, default_tolerance, geographic
from .progress import Found, Stage, staged

if t.TYPE_CHECKING:
    import topology_checker as tc

# The layer that each utility writes
UTILITY_LAYERS = {
    'merge-lines': 'merged_line',
    'explode-lines': 'exploded_line',
}
# What the executable says when a tolerance is in degrees
ANGULAR = 'tolerance is in angular units; calculations are planar'
# How often a run that waits for a rule looks whether it was canceled
POLL_SECONDS = 0.1

Skips = t.Mapping[object, t.Mapping[int, str]]


def rules() -> tuple[Rule, ...]:
    """The rules of the package that the plugin can list."""
    import topology_checker as tc

    found = []
    for rule in tc.Rule:
        for variant in rule.variants:
            layer, other = variant.inputs
            if known(variant.value):
                found.append(
                    Rule(
                        variant.value,
                        variant.description,
                        layer.value,
                        None if other is None else other.value,
                    )
                )
    return tuple(found)


@dataclasses.dataclass(frozen=True)
class Snapshot:
    """The features of an input that go to the package, in its CRS, and those
    that were left out because the package would reject them.

    Unless it is checked, the input is read as it is, like a layer that the
    executable reads from its own file, and the package rejects what it
    cannot take. A checked input leaves out what it would reject, like an
    export for the executable.
    """

    item: Input
    crs: QgsCoordinateReferenceSystem
    fids: list[int]
    data: list[bytes]
    skipped: tuple[Skipped, ...]
    checked: bool
    # The file of the layer, which names the input as the executable would
    # read it from there
    dataset: Dataset | None
    # What was not given to the package, as the executable counts it
    nulls: int = 0
    empties: int = 0
    flattened: int = 0

    def notes(self, name: str) -> list[str]:
        """What the executable says about the features that it does not
        read, and about a missing CRS."""
        counts = (
            (self.nulls, 'skipped {} null geometries'),
            (self.empties, 'skipped {} empty geometries'),
            (self.flattened, 'dropped Z/M values from {} geometries'),
        )
        found = [f'layer {name}: {text.format(n)}' for n, text in counts if n]
        if not self.crs.isValid():
            found.append(f'layer {name}: missing CRS')
        return found


def flatten(geometry: QgsGeometry) -> bool:
    """Drops the Z and M values of a geometry, which the package does not
    read, and says whether it had any."""
    kind = geometry.wkbType()
    if not QgsWkbTypes.hasZ(kind) and not QgsWkbTypes.hasM(kind):
        return False
    part = not_none(geometry.get())
    part.dropZValue()
    part.dropMValue()
    return True


def read(
    item: Input,
    target: QgsCoordinateReferenceSystem | None,
    checked: bool,
    skip: t.Mapping[int, str],
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
    exported: bool | None = None,
) -> Snapshot:
    """The features of the input for the package, in the target CRS.

    Exported says whether the executable would read an export of the input
    rather than its file, which by default is whether the input is checked.
    """
    require_readable(item)
    # A connection that QGIS broke earlier hides features from the run, and
    # from the map
    reconnect(item)
    transform = reprojection(item, target, context, feedback)
    crs = (
        item.source.sourceCrs()
        if transform is None
        else transform.destinationCrs()
    )
    fids: list[int] = []
    wkbs: list[bytes] = []
    skipped: list[Skipped] = []
    nulls = empties = flattened = 0
    for fid, geometry in geometries(item, transform, feedback):
        if geometry.isNull():
            # QGIS exports a missing geometry as an empty one, which is how
            # the executable then counts it
            if checked if exported is None else exported:
                empties += 1
            else:
                nulls += 1
            continue
        # QGIS takes a point with one coordinate that is not a number for an
        # empty one, which the executable rejects
        if geometry.isEmpty() and not not_finite(geometry):
            empties += 1
            continue
        reason = skip.get(fid)
        if reason is None and checked:
            reason = invalid_reason(geometry)
        if reason is not None:
            skipped.append(left_out(item, fid, geometry, reason, context))
            continue
        geometry = straight(geometry)
        flattened += flatten(geometry)
        fids.append(fid)
        # data() is bytes at run time; some stubs type it as str
        wkbs.append(t.cast('bytes', geometry.asWkb().data()))
    return Snapshot(
        item,
        crs,
        fids,
        wkbs,
        tuple(skipped),
        checked,
        None if checked else own_file(item, target),
        nulls,
        empties,
        flattened,
    )


@dataclasses.dataclass(frozen=True)
class Registered:
    """An input as a layer of the package."""

    key: object
    snapshot: Snapshot
    name: str
    layer: tc.LayerId


def register(
    topology: tc.Topology, key: object, snapshot: Snapshot, name: str
) -> Registered:
    """Gives the features to the package, which raises a Rejected for one
    that it refuses."""
    import topology_checker as tc

    item = snapshot.item
    try:
        layer = topology.add_layer(
            snapshot.data,
            name=name,
            fids=snapshot.fids,
            kind=tc.GeometryKind(item.kind),
        )
    except ValueError as error:
        failure = parse_failure(str(error))
        if failure is None:
            raise RunError(f'{item.name}: {error}') from error
        _, fid, reason = failure
        rejected = Rejected(item.name, fid, reason, key)
        rejected.as_is = not snapshot.checked
        rejected.skippable = rejected.skippable and fid in snapshot.fids
        raise rejected from error
    return Registered(key, snapshot, name, layer)


def snapshots(
    jobs: t.Sequence[Job],
    skips: Skips,
    exported: set[object],
    made: dict[object, tuple[object, Snapshot]],
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
) -> dict[object, Snapshot]:
    """The features of each input, in the CRS of the first, read again only
    for an input whose features to leave out changed."""
    target = jobs[0].layer.source.sourceCrs()
    found: dict[object, Snapshot] = {}
    items = [
        item
        for job in jobs
        for item in (job.layer, job.other)
        if item is not None
    ]
    for item in items:
        key = item.key
        if key in found:
            continue
        skip = skips.get(key, {})
        checked = key in exported or own_file(item, target) is None
        how = (checked, tuple(sorted(skip.items())))
        if key in made and made[key][0] == how:
            found[key] = made[key][1]
        else:
            found[key] = read(item, target, checked, skip, context, feedback)
            made[key] = (how, found[key])
        feedback.setProgress(50 * len(found) / len(items))
    return found


def run_once(
    jobs: t.Sequence[Job],
    tolerance: float | None,
    output: str,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
    skips: Skips,
    exported: set[object],
    made: dict[object, tuple[object, Snapshot]],
) -> Outcome:
    """Runs the jobs with the package, and writes the errors to the
    GeoPackage at the output as the executable does.

    The inputs are named as for the executable: after their file where it
    would read them from there, else after the layer.
    """
    import topology_checker as tc

    found = snapshots(jobs, skips, exported, made, context, feedback)
    target = jobs[0].layer.source.sourceCrs()
    crs = (
        target
        if all(snapshot.crs.isValid() for snapshot in found.values())
        else QgsCoordinateReferenceSystem()
    )
    if tolerance is None:
        tolerance = default_tolerance(crs)
    topology = tc.Topology(tolerance)
    taken: set[str] = set()
    # An input that two layers of the project read from the same file is
    # one layer of the package, as it is one of the executable
    registered: dict[object, Registered] = {}
    by_key: dict[object, Registered] = {}
    for key, snapshot in found.items():
        dataset = snapshot.dataset
        who = key if dataset is None else dataset
        if who not in registered:
            name = unique(
                stem(snapshot.item.name) if dataset is None else dataset.name,
                taken,
            )
            registered[who] = register(topology, key, snapshot, name)
        by_key[key] = registered[who]
    for job in jobs:
        add_rule(topology, job, by_key)
    layers = {
        entry.name: layer_id
        for key, entry in by_key.items()
        if (layer_id := found[key].item.layer_id) is not None
    }
    errors = validate(
        topology,
        registered.values(),
        jobs,
        layers,
        feedback,
        ErrorsFile(output, crs, context),
    )
    for warning in errors.warnings + [
        note
        for entry in registered.values()
        for note in entry.snapshot.notes(entry.name)
    ]:
        feedback.pushInfo(f'warning: {warning}')
    if geographic(crs) and tolerance > 0:
        feedback.pushInfo(f'warning: {ANGULAR}')
    feedback.setProgress(100)
    return Outcome(
        errors.counts,
        layers,
        [
            Invalid(entry.name, skipped.fid, skipped.geometry, skipped.reason)
            for entry in registered.values()
            for skipped in entry.snapshot.skipped
        ],
        target,
        errors.written,
    )


def add_rule(
    topology: tc.Topology, job: Job, by_key: t.Mapping[object, Registered]
) -> None:
    """Adds the rule of the job, raising unless the package runs that very
    rule."""
    import topology_checker as tc

    rule = next(
        (
            rule
            for rule in tc.Rule
            for variant in rule.variants
            if variant.value == job.rule.name
        ),
        None,
    )
    if rule is None:
        raise RunError(
            tr('The Python package has no rule {rule}').format(
                rule=job.rule.name
            )
        )
    other = None if job.other is None else by_key[job.other.key].layer
    try:
        spec = topology.add_rule(rule, by_key[job.layer.key].layer, other=other)
    except ValueError as error:
        raise RunError(f'{job.rule.name}: {error}') from error
    if spec.resolved_rule.value != job.rule.name:
        raise RunError(
            tr('The Python package runs {found} for {rule}').format(
                found=spec.resolved_rule.value, rule=job.rule.name
            )
        )


class Canceled(Exception):
    """The run was canceled while a rule was checked."""


class Reports:
    """The report of each rule of the topology as soon as the rule is done.

    The package checks the rules outside the lock of Python, in a thread
    that waits for them, so that a canceled run stops at once: the stream is
    closed, which stops the rule that is checked then and drops it.
    """

    def __init__(self, topology: tc.Topology) -> None:
        self.found: queue.Queue[object] = queue.Queue()
        self.stopped = threading.Event()
        self.summary: tc.ReportSummary | None = None
        self.ended = False
        self.stream = stream = topology.iter_reports()
        self.thread = threading.Thread(
            target=self.read, args=(stream,), name='topology-reports'
        )
        self.thread.daemon = True
        self.thread.start()

    def read(self, stream: tc.ReportStream) -> None:
        try:
            with stream:
                for report in stream:
                    if self.stopped.is_set():
                        return
                    self.found.put(report)
                    # Not kept while the next rule is checked
                    del report
                self.found.put(Ended(stream.summary))
        except BaseException as error:
            # Raised where the reports are read
            self.found.put(error)

    def next(self, feedback: QgsProcessingFeedback) -> tc.RuleReport | None:
        """The next report, or None when all rules are done. Raises
        Canceled when the run is canceled first."""
        while not self.ended:
            if feedback.isCanceled():
                raise Canceled
            try:
                item = self.found.get(timeout=POLL_SECONDS)
            except queue.Empty:
                continue
            if isinstance(item, Ended):
                self.ended = True
                self.summary = item.summary
            elif isinstance(item, BaseException):
                self.ended = True
                raise item
            else:
                return t.cast('tc.RuleReport', item)
        return None

    def ready(self) -> bool:
        """Whether the next report is at hand already."""
        return not self.found.empty()

    def __enter__(self) -> Reports:
        return self

    def __exit__(self, *_: object) -> None:
        self.stopped.set()
        if self.ended:
            self.close()
            return
        # Stopped first: the rule that is checked stops at once, and the run
        # ends without waiting for the package, which can take seconds to
        # stop a large rule. A daemon thread that is still in the package
        # when Python exits is ended inside native code; this goes once the
        # package stops any rule within a fraction of a second
        closing = threading.Thread(
            target=self.close, name='topology-reports-close'
        )
        closing.daemon = True
        closing.start()

    def close(self) -> None:
        with contextlib.suppress(Exception):
            self.stream.close()


@dataclasses.dataclass(frozen=True)
class Ended:
    summary: tc.ReportSummary | None


def validate(
    topology: tc.Topology,
    registered: c.Iterable[Registered],
    jobs: t.Sequence[Job],
    layers: t.Mapping[str, str],
    feedback: QgsProcessingFeedback,
    file: ErrorsFile,
) -> Errors:
    """Runs the rules, telling the feedback of each as soon as it is done,
    and writing its errors to the file.

    A canceled run stops at once, with the errors of the rules that are
    done and the warnings known by then. When no rule is done, it raises.
    """
    import topology_checker as tc

    names = {entry.layer: entry.name for entry in registered}
    gathered = Gathered(jobs, layers, feedback, file)
    if feedback.isCanceled():
        raise RunError(tr('Canceled'))
    staged(feedback, Stage.CHECKING, 0, len(jobs))
    index = 0
    try:
        with Reports(topology) as stream:
            while (report := stream.next(feedback)) is not None:
                found: list[Found] = []
                for violation in report.violations:
                    if feedback.isCanceled():
                        raise Canceled
                    values = attributes(index, violation, names)
                    index += 1
                    found += found_in(from_wkb(violation.wkb), values)
                count = report.error_count
                # Neither the report nor its rows are kept once the rule is
                # told and written: a later failure keeps them in its
                # traceback otherwise
                del report
                gathered.add(found, count, stream.ready())
                del found
            summary = not_none(stream.summary)
    except Canceled:
        if not gathered.counts:
            raise RunError(tr('Canceled')) from None
        return gathered.errors(topology.warnings())
    except (ValueError, tc.PrecisionError) as error:
        raise RunError(str(error)) from error
    return gathered.errors(summary.warnings)


def attributes(
    index: int,
    violation: tc.Violation,
    names: t.Mapping[tc.LayerId, str],
) -> tuple[object, ...]:
    """The columns of an error, in the order of FIELDS."""
    values: list[object] = [
        index,
        violation.resolved_rule.value,
        violation.error_type.value,
    ]
    for feature in (violation.origin, violation.destination):
        values += (
            [None, None, None]
            if feature is None
            else [names[feature.layer], feature.fid, feature.part]
        )
    return tuple(values)


def run_utility(
    command: str,
    item: Input,
    output: str,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
) -> tuple[Skipped, ...]:
    """Merges or explodes the lines of the input as the executable does, and
    writes them to the layer `merged_line` or `exploded_line` of the
    GeoPackage at the output. Features that the package would reject are
    left out, which it returns."""
    import topology_checker as tc

    # The executable reads the file of the layer where it can, as it is
    exported = own_file(item, None) is None
    snapshot = read(item, None, True, {}, context, feedback, exported)
    name = stem(item.name)
    tolerance = default_tolerance(snapshot.crs)
    topology = tc.Topology(tolerance)
    # The layer says what the package would reject, and what it leaves out
    register(topology, item.key, snapshot, name)
    try:
        if command == 'merge-lines':
            made = tc.merge_lines(snapshot.data, tolerance)
        else:
            made = tc.explode_lines(snapshot.data)
    except (ValueError, tc.PrecisionError) as error:
        raise RunError(str(error)) from error
    for warning in [*topology.warnings(), *snapshot.notes(name)]:
        feedback.pushInfo(f'warning: {warning}')
    if geographic(snapshot.crs) and tolerance > 0:
        feedback.pushInfo(f'warning: {ANGULAR}')
    layer = UTILITY_LAYERS[command]
    fields = QgsVectorLayer('None', 'fields', 'memory').fields()
    writer = create_layer(
        output,
        layer,
        fields,
        MULTI['line'],
        snapshot.crs,
        QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteFile,
        context,
    )
    try:
        write_rows(
            writer, ((from_wkb(line), []) for line in made), fields, layer
        )
    finally:
        del writer
    feedback.setProgress(100)
    return snapshot.skipped
