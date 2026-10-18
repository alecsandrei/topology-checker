from __future__ import annotations

import typing as t

from qgis.core import QgsApplication, QgsTask
from qgis.PyQt import sip
from qgis.PyQt.QtCore import QCoreApplication

from ..core import binary
from ..core.binary import Rule
from ..exceptions import ExecutableError
from ..utils import named, not_none

if t.TYPE_CHECKING:
    from .dock import Dock


def tr(text: str) -> str:
    return QCoreApplication.translate('TopologyRules', text)


def refresh(dock: Dock) -> None:
    """Lists the rules of the executable, or says why there are none.

    An executable that was asked before answers at once. One that was not can
    take seconds when it starts for the first time, so the dock says that it
    is checking and fills in when the answer comes.
    """
    try:
        path, modified = binary.locate()
    except ExecutableError as error:
        dock.rules_failed(error)
        return
    found = binary.probe_cache.peek(path, modified)
    if found is not None:
        dock.rules_known(found)
        return
    if dock.probing == (path, modified):
        return
    dock.probing = (path, modified)
    dock.available = False
    dock.form.set_rules(())
    dock.setup_page.checking()
    dock.update_buttons()
    dock.probe_task = named(QgsTask, 'fromFunction')(
        tr('Checking topology-checker'),
        lambda task: ask(path, modified),
        on_finished=lambda error, rules: answered(dock, error, rules),
    )
    not_none(QgsApplication.taskManager()).addTask(dock.probe_task)


def ask(path: str, modified: int) -> tuple[Rule, ...] | Exception:
    """The rules of the executable, or why there are none. An error is
    returned and not raised, as QGIS 4 does not call back a task whose
    function raised."""
    try:
        return binary.probe(path, modified)
    except Exception as error:
        return error


def answered(
    dock: Dock,
    error: Exception | None,
    result: tuple[Rule, ...] | Exception | None,
) -> None:
    """The answer of the executable, back on the main thread."""
    if sip.isdeleted(dock):
        return
    dock.probing = None
    dock.probe_task = None
    failure = error or (result if isinstance(result, Exception) else None)
    if isinstance(failure, ExecutableError):
        dock.rules_failed(failure)
    elif failure is not None or result is None:
        dock.rules_failed(ExecutableError(str(failure or 'No answer')))
    else:
        dock.rules_known(t.cast('tuple[Rule, ...]', result))
