from __future__ import annotations

import typing as t

from qgis.PyQt import sip

from .. import probing
from ..core import binary
from ..core.binary import Rule
from ..exceptions import ExecutableError

if t.TYPE_CHECKING:
    from .dock import Dock


def refresh(dock: Dock) -> None:
    """Lists the rules of the executable, or says why there are none.

    An executable that was asked before answers at once. One that was not can
    take seconds when it starts for the first time, so the dock says that it
    is checking and fills in when the answer comes.
    """
    try:
        path, modified = binary.locate()
    except ExecutableError as error:
        # No answer of an executable that was asked is wanted any more
        dock.probing = None
        dock.rules_failed(error)
        return
    found = binary.probe_cache.peek(path, modified)
    if found is not None:
        dock.probing = None
        dock.rules_known(found)
        return
    if dock.probing == (path, modified):
        return
    dock.probing = (path, modified)
    dock.available = False
    dock.form.set_rules(())
    dock.setup_page.checking()
    dock.update_buttons()
    dock.probe_task = probing.ask_in_task(
        path,
        modified,
        lambda rules, error: answered(dock, (path, modified), rules, error),
    )


def answered(
    dock: Dock,
    asked: tuple[str, int],
    rules: tuple[Rule, ...] | None,
    error: Exception | None,
) -> None:
    """The answer of the executable, back on the main thread. One for an
    executable that is not the one in question any more is of no use."""
    if sip.isdeleted(dock) or dock.probing != asked:
        return
    dock.probing = None
    dock.probe_task = None
    if rules is not None:
        dock.rules_known(rules)
    elif isinstance(error, ExecutableError):
        dock.rules_failed(error)
    else:
        dock.rules_failed(ExecutableError(str(error or 'No answer')))
