from __future__ import annotations

import collections.abc as c
import dataclasses
import itertools
import re
import typing as t
from pathlib import Path

from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsFeature,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsVectorLayer,
)

from ..exceptions import RunError
from ..utils import not_none, tr
from .binary import Rule
from .commands import (
    ERROR_LAYERS,
    Check,
    rule_counts,
    run_validate,
    write_config,
)
from .engine import Package, Tool, describe
from .inputs import Prepared, prepare
from .invalid import REJECTED, Invalid, Rejected, parse_failure
from .layers import Input

# The columns of an error that name a feature, and the layer it is in
ID_COLUMNS = (
    ('origin_layer', 'origin_fid'),
    ('destination_layer', 'destination_fid'),
)
# What the tool says when it cannot read a geometry or its envelope in a file
# (crates/io), without saying which feature
UNREADABLE = re.compile(r'WKB|GeoPackage envelope|non-finite')
# How often a run is repeated without the features that the tool rejects
MAX_ATTEMPTS = 25

IdMap = dict[str, tuple[int, ...]]
# The features to leave out of each input, and why
Skips = dict[object, dict[int, str]]


@dataclasses.dataclass(frozen=True)
class Job:
    """A rule to run on one or two inputs."""

    rule: Rule
    layer: Input
    other: Input | None = None


@dataclasses.dataclass(frozen=True)
class Outcome:
    """The errors of each job, in the order of the jobs.

    The ids tell, for each layer that was exported, the id of the source
    feature that the n-th feature of the export came from. The layers tell
    which layer of the project each name in the errors stands for. The
    invalid features are those that were left out of the run.
    """

    counts: list[int]
    ids: IdMap
    layers: dict[str, str]
    invalid: list[Invalid]
    crs: QgsCoordinateReferenceSystem


def run_jobs(
    jobs: t.Sequence[Job],
    tool: Tool,
    tolerance: float | None,
    output: str,
    directory: Path,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
) -> Outcome:
    """Runs the jobs in one pass of the package or the executable, which
    writes the errors to the GeoPackage at the output. All data is brought to
    the CRS of the first input, which is also the CRS of the errors."""
    feedback.pushInfo(describe(tool))
    made: dict[object, t.Any] = {}
    if isinstance(tool, Package):
        from . import package

        def attempt(skips: Skips, exported: set[object]) -> Outcome:
            return package.run_once(
                jobs,
                tolerance,
                output,
                context,
                feedback,
                skips,
                exported,
                made,
            )

    else:
        folders = itertools.count()

        def attempt(skips: Skips, exported: set[object]) -> Outcome:
            folder = directory / f'run{next(folders)}'
            folder.mkdir()
            return run_once(
                jobs,
                tool,
                tolerance,
                output,
                folder,
                context,
                feedback,
                skips,
                exported,
                made,
            )

    outcome = retried(jobs, attempt, feedback)
    if outcome.invalid:
        feedback.pushWarning(
            tr(
                '{count} features with an invalid geometry were left out of '
                'the run'
            ).format(count=len(outcome.invalid))
        )
    return outcome


def retried(
    jobs: t.Sequence[Job],
    attempt: c.Callable[[Skips, set[object]], Outcome],
    feedback: QgsProcessingFeedback,
) -> Outcome:
    """The outcome of the first attempt that the tool does not stop.

    Layers go to the tool as they are where they can. If the tool rejects a
    feature, the run is repeated without the features that it would reject,
    which are reported as errors: the exported inputs leave them out. Only
    the input that held the feature is exported again: the others stay as
    they were.
    """
    skips: Skips = {}
    # The inputs that are exported and not read from their own file
    exported: set[object] = set()
    first: Rejected | None = None
    for _ in range(MAX_ATTEMPTS):
        try:
            return attempt(skips, exported)
        except Rejected as rejected:
            # The first feature that was named is the one that is reported
            # when skipping features does not help; when the tool stops on
            # something else, that is what stopped the run. The features
            # that were left out before are in the log
            first = first or rejected
            if not rejected.skippable:
                if rejected.reason.startswith(REJECTED):
                    raise first from rejected
                raise
            if rejected.as_is:
                exported.add(rejected.key)
            elif rejected.fid in skips.get(rejected.key, {}):
                raise first from rejected
            else:
                skips.setdefault(rejected.key, {})[rejected.fid] = (
                    rejected.reason
                )
                feedback.pushInfo(
                    tr('Left out of the run: {reason}').format(reason=rejected)
                )
        except RunError as error:
            # The tool cannot read a geometry of a file (crates/io codec),
            # and does not say which: the export leaves out what it would
            # reject
            keys = {
                item.key
                for job in jobs
                for item in (job.layer, job.other)
                if item is not None
            }
            if not UNREADABLE.search(str(error)) or keys <= exported:
                raise
            exported.update(keys)
    raise first or RunError(tr('Too many features were rejected by the tool'))


def run_once(
    jobs: t.Sequence[Job],
    executable: str,
    tolerance: float | None,
    output: str,
    directory: Path,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
    skips: Skips,
    exported: set[object],
    made: dict[object, tuple[object, Prepared]],
) -> Outcome:
    """Runs the jobs once with the executable, from the layers' own files
    and from exports."""
    target = jobs[0].layer.source.sourceCrs()
    prepared: dict[object, Prepared] = {}
    names: dict[object, str] = {}
    inputs: dict[object, Input] = {}

    def get(item: Input) -> Prepared:
        if item.key not in prepared:
            # What was made for the same skips and way is made again for no
            # one
            how = (
                item.key in exported,
                tuple(sorted(skips.get(item.key, {}).items())),
            )
            if item.key in made and made[item.key][0] == how:
                prepared[item.key] = made[item.key][1]
            else:
                prepared[item.key] = prepare(
                    item,
                    directory,
                    target,
                    context,
                    feedback,
                    skips.get(item.key),
                    item.key not in exported,
                )
                made[item.key] = (how, prepared[item.key])
            names[item.key] = item.name
            inputs[item.key] = item
        return prepared[item.key]

    checks = [
        Check(
            job.rule,
            get(job.layer).dataset,
            None if job.other is None else get(job.other).dataset,
        )
        for job in jobs
    ]
    config = directory / 'checks.toml'
    aliases = write_config(config, checks)
    completed = run_validate(
        executable, str(config), tolerance, output, feedback
    )
    try:
        counts = rule_counts(completed)
    except RunError as error:
        failure = parse_failure(str(error))
        for key, item in prepared.items():
            if failure is not None and aliases[item.dataset] == failure[0]:
                fid = source_id(
                    {aliases[item.dataset]: item.ids or ()}
                    if item.ids is not None
                    else {},
                    failure[0],
                    failure[1],
                )
                rejected = Rejected(
                    names[key], int(t.cast(int, fid)), failure[2], key
                )
                rejected.as_is = item.ids is None
                # A number outside the export names no feature of the layer
                if item.ids is not None and not 1 <= failure[1] <= len(
                    item.ids
                ):
                    rejected.skippable = False
                raise rejected from error
        raise
    invalid = [
        Invalid(
            aliases[item.dataset],
            skipped.fid,
            skipped.geometry,
            skipped.reason,
        )
        for item in prepared.values()
        for skipped in item.skipped
    ]

    return Outcome(
        counts,
        {
            aliases[item.dataset]: item.ids
            for item in prepared.values()
            if item.ids is not None
        },
        {
            aliases[item.dataset]: layer_id
            for key, item in prepared.items()
            if (layer_id := inputs[key].layer_id) is not None
        },
        invalid,
        target,
    )


def source_id(ids: IdMap, layer: object, fid: object) -> object:
    """The id of the source feature that an error names in a layer."""
    table = ids.get(str(layer))
    if table is None or not isinstance(fid, int):
        return fid
    return table[fid - 1] if 1 <= fid <= len(table) else fid


def renumber_errors(path: str, ids: IdMap) -> None:
    """Gives the errors of a GeoPackage the ids of the source features."""
    if not ids:
        return
    for name in ERROR_LAYERS:
        layer = QgsVectorLayer(f'{path}|layername={name}', name, 'ogr')
        if not layer.isValid():
            continue
        fields = layer.fields()
        changes: dict[int, dict[int, object]] = {}
        for feature in t.cast('c.Iterable[QgsFeature]', layer.getFeatures()):
            new: dict[int, object] = {}
            for layer_column, id_column in ID_COLUMNS:
                if fields.indexOf(id_column) < 0:
                    continue
                before = feature[id_column]
                after = source_id(ids, feature[layer_column], before)
                if isinstance(before, int) and after != before:
                    new[fields.indexOf(id_column)] = after
            if new:
                changes[feature.id()] = new
        if changes and not not_none(layer.dataProvider()).changeAttributeValues(
            changes
        ):
            raise RunError(
                tr('Could not write the feature ids to {path}').format(
                    path=path
                )
            )
