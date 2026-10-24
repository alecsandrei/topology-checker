from __future__ import annotations

import collections.abc as c

from qgis.core import QgsApplication, QgsTask

from .core import binary
from .core.binary import Rule
from .exceptions import ExecutableError
from .utils import named, not_none, tr


def ask(path: str, modified: int) -> tuple[Rule, ...] | Exception:
    """The rules of the executable, or why there are none. An error is
    returned and not raised, as QGIS 4 does not call back a task whose
    function raised."""
    try:
        return binary.probe(path, modified)
    except Exception as error:
        return error


def ask_in_task(
    path: str,
    modified: int,
    answered: c.Callable[[tuple[Rule, ...] | None, Exception | None], None],
) -> QgsTask:
    """Asks the executable for its rules in a task, and calls back on the
    main thread with the rules or with why there are none. The caller keeps
    the task: Python drops the callback of a task that it collects."""

    def finished(
        error: Exception | None, result: tuple[Rule, ...] | Exception | None
    ) -> None:
        failure = error or (result if isinstance(result, Exception) else None)
        if failure is None and result is None:
            failure = ExecutableError(tr('No answer'))
        rules = None if failure is not None else result
        answered(rules if isinstance(rules, tuple) else None, failure)

    task = named(QgsTask, 'fromFunction')(
        tr('Checking topology-checker'),
        lambda task: ask(path, modified),
        on_finished=finished,
    )
    not_none(QgsApplication.taskManager()).addTask(task)
    return task
