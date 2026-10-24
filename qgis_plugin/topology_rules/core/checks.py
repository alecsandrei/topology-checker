from __future__ import annotations

import collections.abc as c
import contextlib
import dataclasses
import itertools
import re
import typing as t
from pathlib import Path

from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsProcessingContext,
    QgsProcessingFeedback,
)

from ..enums import ExitCode
from ..exceptions import RunError
from ..utils import tr
from .binary import Rule
from .commands import Check, run_validate, streams, write_config
from .engine import Package, Tool, describe
from .errors import ErrorsFile, Gathered, remove_file
from .inputs import Prepared, prepare
from .invalid import REJECTED, Invalid, Rejected, parse_failure
from .layers import Dataset, Input
from .progress import Stage, staged
from .report import IdMap, Report, source_id
from .runner import error_message

# What the tool says when it cannot read a geometry or its envelope in a file
# (crates/io), without saying which feature
UNREADABLE_GEOMETRY = re.compile(r'WKB|GeoPackage envelope|non-finite')
# How often a run is repeated without the features that the tool rejects
MAX_ATTEMPTS = 25
# How much of the output that is not of the report the log shows
SHOWN_CHARS = 500

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
    """The errors of each job that is done, in the order of the jobs: fewer
    than the jobs when the run was canceled.

    The layers tell which layer of the project each name in the errors
    stands for. The invalid features are those that were left out of the
    run.
    """

    counts: list[int]
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
    """Runs the jobs in one pass of the package or the executable, and
    writes the errors to the GeoPackage at the output, rule after rule. All
    data is brought to the CRS of the first input, which is also the CRS of
    the errors. A run that fails leaves no file."""
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

    try:
        outcome = retried(jobs, attempt, feedback)
    except BaseException:
        # What the rules before the failure wrote is not a run
        with contextlib.suppress(RunError):
            remove_file(output)
        raise
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
    they were. Each attempt starts by reading the layers, which is the stage
    that the feedback hears of first, once for each attempt.
    """
    skips: Skips = {}
    # The inputs that are exported and not read from their own file
    exported: set[object] = set()
    first: Rejected | None = None
    for _ in range(MAX_ATTEMPTS):
        staged(feedback, Stage.READING)
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
            if not UNREADABLE_GEOMETRY.search(str(error)) or keys <= exported:
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
    and from exports, and writes the errors to the GeoPackage at the output
    as the package does. The log and the feedback hear of each rule as soon
    as the executable says it is done, and its errors are written then.

    A canceled run ends with the errors of the rules that are done. When
    none is, it raises.
    """
    target = jobs[0].layer.source.sourceCrs()
    prepared: dict[object, Prepared] = {}
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
    layers = {
        aliases[item.dataset]: layer_id
        for key, item in prepared.items()
        if (layer_id := inputs[key].layer_id) is not None
    }
    ids: IdMap = {
        aliases[item.dataset]: item.ids
        for item in prepared.values()
        if item.ids is not None
    }
    # The errors have no CRS where an input has none, as the tool reads it
    crs = (
        target
        if all(item.source.sourceCrs().isValid() for item in inputs.values())
        else QgsCoordinateReferenceSystem()
    )
    gathered = Gathered(
        jobs, layers, feedback, ErrorsFile(output, crs, context)
    )
    names = [
        (
            aliases[check.layer],
            None if check.other is None else aliases[check.other],
        )
        for check in checks
    ]
    report = Report(gathered, ids, names)
    # Which rule runs is not known when the executable says it at the end
    staged(
        feedback,
        Stage.CHECKING,
        0 if streams(executable) else None,
        len(jobs),
    )
    try:
        completed = run_validate(
            executable,
            str(config),
            tolerance,
            feedback,
            report.line,
            lambda: staged(feedback, Stage.CHECKING, None, len(jobs)),
            done=lambda: report.over,
        )
    except RunError:
        if not (feedback.isCanceled() and gathered.counts):
            raise
    else:
        # A tool that was ended once it had told all has no code of its own
        if not completed.ended and completed.exit_code not in (
            ExitCode.VALID.value,
            ExitCode.VIOLATIONS.value,
        ):
            # The report on the output, or the start of it, is not the error
            raise rejected(
                RunError(
                    error_message(dataclasses.replace(completed, stdout=''))
                ),
                prepared,
                aliases,
                inputs,
            )
        if not report.complete:
            unread(feedback, report, completed.stdout)
            raise RunError(tr("Could not read the executable's report."))
    errors = gathered.errors(report.warnings or ())
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
    return Outcome(errors.counts, layers, invalid, target)


def unread(
    feedback: QgsProcessingFeedback, report: Report, output: str
) -> None:
    """Tells the log what the report lacks, and the end of the output that
    is not of it."""
    told, total = len(report.gathered.counts), len(report.gathered.jobs)
    if told < total:
        feedback.pushInfo(
            tr('The report has {told} of {total} rules').format(
                told=told, total=total
            )
        )
    else:
        feedback.pushInfo(tr('The report has no last line'))
    rest = output.strip()
    if rest:
        if len(rest) > SHOWN_CHARS:
            rest = '…' + rest[-SHOWN_CHARS:]
        feedback.pushInfo(
            tr('The rest of the output: {text}').format(text=rest)
        )


def rejected(
    error: RunError,
    prepared: c.Mapping[object, Prepared],
    aliases: c.Mapping[Dataset, str],
    inputs: c.Mapping[object, Input],
) -> RunError:
    """The feature that the tool rejected, named by its source id, if the
    error is about one; else the error."""
    failure = parse_failure(str(error))
    if failure is None:
        return error
    alias, fid, reason = failure
    if alias is None:
        return error
    for key, item in prepared.items():
        if aliases[item.dataset] != alias:
            continue
        table = {} if item.ids is None else {alias: item.ids}
        found = Rejected(
            inputs[key].name,
            int(t.cast(int, source_id(table, alias, fid))),
            reason,
            key,
        )
        found.as_is = item.ids is None
        # A number outside the export names no feature of the layer
        if item.ids is not None and not 1 <= fid <= len(item.ids):
            found.skippable = False
        found.__cause__ = error
        return found
    return error
