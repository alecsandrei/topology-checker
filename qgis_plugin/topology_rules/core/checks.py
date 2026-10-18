from __future__ import annotations

import collections.abc as c
import dataclasses
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
from ..utils import not_none
from .binary import Rule
from .commands import (
    ERROR_LAYERS,
    Check,
    rule_counts,
    validate_command,
    write_config,
)
from .inputs import Prepared, prepare
from .invalid import Invalid, Rejected, parse_failure
from .layers import Input
from .runner import execute

# The columns of an error that name a feature, and the layer it is in
ID_COLUMNS = (
    ('origin_layer', 'origin_fid'),
    ('destination_layer', 'destination_fid'),
)
# How often a run is repeated without the features that the tool rejects
MAX_ATTEMPTS = 25

IdMap = dict[str, tuple[int, ...]]


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
    executable: str,
    tolerance: float | None,
    output: str,
    directory: Path,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
) -> Outcome:
    """Runs the jobs in one pass of the tool, which writes the errors to the
    GeoPackage at the output. All data is brought to the CRS of the first
    input, which is also the CRS of the errors.

    Layers go to the tool as they are where they can. If the tool rejects a
    feature, the run is repeated from exports that leave out the features
    that it would reject, which are reported as errors.
    """
    skips: dict[object, dict[int, str]] = {}
    direct = True
    first: Rejected | None = None
    for attempt in range(MAX_ATTEMPTS):
        folder = directory / f'run{attempt}'
        folder.mkdir()
        try:
            outcome = run_once(
                jobs,
                executable,
                tolerance,
                output,
                folder,
                context,
                feedback,
                skips,
                direct,
            )
        except Rejected as rejected:
            # The first feature that was named is the one that is reported
            first = first or rejected
            if not rejected.skippable:
                raise first from rejected
            if direct:
                direct = False
            elif rejected.fid in skips.get(rejected.key, {}):
                raise first from rejected
            else:
                skips.setdefault(rejected.key, {})[rejected.fid] = (
                    rejected.reason
                )
            continue
        if outcome.invalid:
            feedback.pushWarning(
                f'{len(outcome.invalid)} features with an invalid geometry '
                'were left out of the run'
            )
        return outcome
    raise first or RunError('Too many features were rejected by the tool')


def run_once(
    jobs: t.Sequence[Job],
    executable: str,
    tolerance: float | None,
    output: str,
    directory: Path,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
    skips: dict[object, dict[int, str]],
    direct: bool,
) -> Outcome:
    target = jobs[0].layer.source.sourceCrs()
    prepared: dict[object, Prepared] = {}
    names: dict[object, str] = {}
    inputs: dict[object, Input] = {}

    def get(item: Input) -> Prepared:
        if item.key not in prepared:
            prepared[item.key] = prepare(
                item,
                directory,
                target,
                context,
                feedback,
                skips.get(item.key),
                direct,
            )
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
    completed = execute(
        validate_command(executable, str(config), tolerance, output), feedback
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
            raise RunError(f'Could not write the feature ids to {path}')
