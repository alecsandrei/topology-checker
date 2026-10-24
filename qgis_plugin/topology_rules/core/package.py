"""Runs the checks inside QGIS with the topology-checker Python package.

The package reads the features as they are in the layers, as WKB, with the
ids of the layers, and the errors are written as the executable writes
them. The package is imported by the run that uses it: the module also
loads where it is not installed.
"""

from __future__ import annotations

import collections.abc as c
import dataclasses
import queue
import threading
import typing as t

from qgis.core import (
    Qgis,
    QgsCoordinateReferenceSystem,
    QgsFeature,
    QgsFields,
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
from .commands import ERROR_LAYERS, unique
from .inputs import (
    BATCH,
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
    write,
)
from .invalid import FIELDS, Invalid, Rejected, parse_failure
from .layers import Dataset, Input, default_tolerance, geographic, wkb_kind
from .progress import Checked, Found, Stage, checked, log_rule, staged

if t.TYPE_CHECKING:
    import topology_checker as tc

# The geometry of each layer that the executable writes
MULTI = {
    'point': Qgis.WkbType.MultiPoint,
    'line': Qgis.WkbType.MultiLineString,
    'polygon': Qgis.WkbType.MultiPolygon,
}
# The layer that each utility writes
UTILITY_LAYERS = {
    'merge-lines': 'merged_line',
    'explode-lines': 'exploded_line',
}
# What the executable says when a tolerance is in degrees
ANGULAR = 'tolerance is in angular units; calculations are planar'
# The id of "Undefined geographic SRS", which the executable gives a layer
# without a CRS
NO_CRS = 'SRID=0'
# The columns of an error as the executable writes them: texts of any
# length and 64-bit integers
COLUMNS = FIELDS.replace(':string', ':string(0)').replace(':integer', ':long')
# The families of the error layers, in the order the panel reads them
FAMILIES = tuple(name.removeprefix('errors_') for name in ERROR_LAYERS)
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
    """Runs the jobs with the package, which writes the errors to the
    GeoPackage at the output as the executable does.

    The inputs are named as for the executable: after their file where it
    would read them from there, else after the layer.
    """
    import topology_checker as tc

    staged(feedback, Stage.READING)
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
    errors = validate(topology, registered.values(), jobs, layers, feedback)
    if errors.rows:
        write_errors(output, errors, crs, context)
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
        {},
        layers,
        [
            Invalid(entry.name, skipped.fid, skipped.geometry, skipped.reason)
            for entry in registered.values()
            for skipped in entry.snapshot.skipped
        ],
        target,
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


@dataclasses.dataclass
class Errors:
    """The errors of a run, as the rows of the layer of each family of
    geometry, and what the package said about the data."""

    rows: dict[str, list[tuple[QgsGeometry, list[object]]]]
    counts: list[int]
    warnings: list[str]


def from_wkb(wkb: bytes) -> QgsGeometry:
    geometry = QgsGeometry()
    geometry.fromWkb(wkb)
    return geometry


def members(geometry: QgsGeometry) -> c.Iterator[QgsGeometry]:
    """The geometry, or each member of a collection, which the executable
    writes to the layer of its family."""
    collection = Qgis.WkbType.GeometryCollection
    if QgsWkbTypes.flatType(geometry.wkbType()) == collection:
        for part in geometry.asGeometryCollection():
            yield from members(part)
    else:
        yield geometry


class Canceled(Exception):
    """The run was canceled while a rule was checked."""


class Reports:
    """The report of each rule of the topology as soon as the rule is done.

    The package checks the rules outside the lock of Python, in a thread
    that waits for them, so that a canceled run stops at once: the rule that
    is checked then is dropped, and the stream is closed when it is done.
    """

    def __init__(self, topology: tc.Topology) -> None:
        self.found: queue.Queue[object] = queue.Queue()
        self.stopped = threading.Event()
        self.summary: tc.ReportSummary | None = None
        self.ended = False
        stream = topology.iter_reports()
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

    def __enter__(self) -> Reports:
        return self

    def __exit__(self, *_: object) -> None:
        self.stopped.set()


@dataclasses.dataclass(frozen=True)
class Ended:
    summary: tc.ReportSummary | None


def validate(
    topology: tc.Topology,
    registered: c.Iterable[Registered],
    jobs: t.Sequence[Job],
    layers: t.Mapping[str, str],
    feedback: QgsProcessingFeedback,
) -> Errors:
    """Runs the rules, telling the feedback of each as soon as it is done.

    A canceled run stops at once, with the errors of the rules that are
    done. When none is, it raises.
    """
    import topology_checker as tc

    names = {entry.layer: entry.name for entry in registered}
    rows: dict[str, list[tuple[QgsGeometry, list[object]]]] = {}
    counts: list[int] = []
    if feedback.isCanceled():
        raise RunError(tr('Canceled'))
    total = len(jobs)
    staged(feedback, Stage.CHECKING, 0, total)
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
                    for member in members(from_wkb(violation.wkb)):
                        family = wkb_kind(member.wkbType())
                        member.convertToMultiType()
                        found.append(Found(family, member, values))
                for item in found:
                    rows.setdefault(item.family, []).append(
                        (item.geometry, t.cast('list[object]', item.values))
                    )
                count = report.error_count
                counts.append(count)
                log_rule(feedback, jobs, len(counts) - 1, count)
                # The errors in the order the panel reads them from the
                # layers: by family, then as they were found
                ordered = sorted(found, key=lambda f: FAMILIES.index(f.family))
                checked(
                    feedback,
                    Checked(
                        len(counts) - 1, total, count, tuple(ordered), layers
                    ),
                )
            summary = not_none(stream.summary)
    except Canceled:
        if not counts:
            raise RunError(tr('Canceled')) from None
        return Errors(rows, counts, [])
    except (ValueError, tc.PrecisionError) as error:
        raise RunError(str(error)) from error
    return Errors(rows, counts, list(summary.warnings))


def attributes(
    index: int,
    violation: tc.Violation,
    names: t.Mapping[tc.LayerId, str],
) -> list[object]:
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
    return values


def create_layer(
    path: str,
    name: str,
    fields: QgsFields,
    kind: Qgis.WkbType,
    crs: QgsCoordinateReferenceSystem,
    first: bool,
    context: QgsProcessingContext,
) -> QgsVectorFileWriter:
    """A new layer of the GeoPackage at the path, which the first one
    creates."""
    options = QgsVectorFileWriter.SaveVectorOptions()
    options.driverName = 'GPKG'
    options.fileEncoding = 'UTF-8'
    options.layerName = name
    options.actionOnExistingFile = (
        QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteFile
        if first
        else QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteLayer
    )
    if not crs.isValid():
        options.layerOptions = [NO_CRS]
    writer = QgsVectorFileWriter.create(
        path, fields, kind, crs, context.transformContext(), options
    )
    if writer is None or (
        writer.hasError() != QgsVectorFileWriter.WriterError.NoError
    ):
        message = '' if writer is None else writer.errorMessage()
        raise RunError(
            tr('Could not write {layer} to {path}: {error}').format(
                layer=name, path=path, error=message
            )
        )
    return writer


def write_rows(
    writer: QgsVectorFileWriter,
    rows: c.Iterable[tuple[QgsGeometry, list[object]]],
    fields: QgsFields,
    name: str,
) -> None:
    """Writes the geometries, as multi geometries, with their columns."""
    batch: list[QgsFeature] = []
    for geometry, values in rows:
        geometry.convertToMultiType()
        feature = QgsFeature(fields)
        feature.setGeometry(geometry)
        feature.setAttributes(values)
        batch.append(feature)
        if len(batch) == BATCH:
            write(writer, batch, name)
    write(writer, batch, name)


def write_errors(
    path: str,
    errors: Errors,
    crs: QgsCoordinateReferenceSystem,
    context: QgsProcessingContext,
) -> None:
    """Writes the errors to the layers `errors_line`, `errors_point` and
    `errors_polygon` of the GeoPackage, those that have any."""
    columns = QgsVectorLayer(f'None?field={COLUMNS}', 'errors', 'memory')
    fields = columns.fields()
    for index, family in enumerate(sorted(errors.rows)):
        name = f'errors_{family}'
        writer = create_layer(
            path, name, fields, MULTI[family], crs, index == 0, context
        )
        try:
            write_rows(writer, errors.rows[family], fields, name)
        finally:
            del writer


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
        True,
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
