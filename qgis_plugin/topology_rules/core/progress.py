"""What a run tells while it goes: the stage it is at, and each rule as soon
as it is done.

The engines tell a feedback that is a Progress, which the panel gives a run.
Any other feedback, such as the one of the Processing toolbox, only gets the
log and the progress."""

from __future__ import annotations

import collections.abc as c
import dataclasses
import typing as t
from enum import Enum

from qgis.core import QgsProcessingFeedback

from ..utils import tr
from .binary import named_with

if t.TYPE_CHECKING:
    from .checks import Job


class Stage(str, Enum):
    READING = 'reading'
    CHECKING = 'checking'
    WRITING = 'writing'


@dataclasses.dataclass(frozen=True, slots=True)
class Found:
    """An error of a rule as a row of the layer of its family: a multi
    geometry, as WKB, which takes a fraction of the memory of a geometry,
    and the columns that the error layers have."""

    family: str
    wkb: bytes
    values: tuple[object, ...]


@dataclasses.dataclass(frozen=True)
class Checked:
    """A rule that is done: its place in the run, how many errors it found,
    and the errors. The layers tell which layer of the project each name in
    the errors stands for."""

    index: int
    total: int
    count: int
    errors: tuple[Found, ...]
    layers: c.Mapping[str, str] = dataclasses.field(default_factory=dict)


class Progress(QgsProcessingFeedback):
    """A feedback that is told the stage of the run and the rules that are
    done, in the thread of the run."""

    def staged(self, stage: Stage, index: int | None, total: int) -> None:
        """The run reads the layers, checks the rule at the index (None
        when it is not known which), or writes the errors."""

    def checked(self, done: Checked) -> None:
        """A rule is done."""


def staged(
    feedback: QgsProcessingFeedback,
    stage: Stage,
    index: int | None = None,
    total: int = 0,
) -> None:
    if isinstance(feedback, Progress):
        feedback.staged(stage, index, total)


def checked(
    feedback: QgsProcessingFeedback, done: Checked, more: bool = False
) -> None:
    """Tells the log and the progress that a rule is done, and the feedback
    of the panel what it found. Then the next rule is checked, or the
    errors are written. With more, the next rule is done already, and is
    told next: the run does not say that it checks it."""
    feedback.setProgress(
        max(feedback.progress(), 50 + 50 * (done.index + 1) / done.total)
    )
    if isinstance(feedback, Progress):
        feedback.checked(done)
        following = done.index + 1
        if following >= done.total:
            feedback.staged(Stage.WRITING, None, done.total)
        elif not more:
            feedback.staged(Stage.CHECKING, following, done.total)


def label(job: Job) -> str:
    """The rule of a job as the errors are grouped under it, such as
    'roads: Must not have dangles'."""
    title = (
        job.rule.short_name
        if job.other is None
        else named_with(job.rule.plain_name, job.other.name)
    )
    return f'{job.layer.name}: {title}'


def counted(count: int) -> str:
    return (
        tr('1 error')
        if count == 1
        else tr('{count} errors').format(count=count)
    )


def log_rule(
    feedback: QgsProcessingFeedback,
    jobs: t.Sequence[Job],
    index: int,
    count: int,
) -> None:
    """The line of the log for a rule that is done: 'roads: Must not have
    dangles: 46 errors'."""
    if 0 <= index < len(jobs):
        feedback.pushInfo(f'{label(jobs[index])}: {counted(count)}')
