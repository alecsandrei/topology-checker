from __future__ import annotations

import collections.abc as c
import contextlib
import json
import typing as t

from qgis.core import QgsProject
from qgis.PyQt.QtCore import QObject

from ..utils import not_none
from .entries import Entry

if t.TYPE_CHECKING:
    from .dock import Dock

# The options of the panel that are saved with the project
OPTIONS = ('selected_only', 'extent_only')
# The part of the project file that is the plugin's
SECTION = 'TopologyRules'


class ProjectState(QObject):
    """Keeps the list of rules and the tolerance in the project, so that they
    come back when the project is opened."""

    def __init__(self, dock: Dock):
        # A child of the dock, so that it stops listening when the dock goes
        super().__init__(dock)
        self.dock = dock
        self.quiet_now = False
        # What the project had when the executable was not there to name the
        # rules, for when it is
        self.pending: list[dict[str, t.Any]] | None = None

    @contextlib.contextmanager
    def quiet(self) -> c.Iterator[None]:
        """Changes the list without writing the change to the project."""
        before = self.quiet_now
        self.quiet_now = True
        try:
            yield
        finally:
            self.quiet_now = before

    def save(self) -> None:
        if self.quiet_now:
            return
        project = not_none(QgsProject.instance())
        project.writeEntry(
            SECTION,
            'rules',
            json.dumps(
                [
                    {
                        'rule': entry.rule.name,
                        'layer': entry.layer,
                        'layer_name': entry.layer_name,
                        'unit': entry.unit,
                        'other': entry.other,
                        'other_name': entry.other_name,
                        'checked': entry.checked,
                    }
                    for entry in self.dock.entries
                ]
            ),
        )
        project.writeEntryDouble(
            SECTION, 'tolerance', self.dock.tolerance.value()
        )
        project.writeEntry(SECTION, 'ignored', self.dock.results.ignored.dump())
        for name in OPTIONS:
            project.writeEntryBool(
                SECTION, name, getattr(self.dock, name).isChecked()
            )

    def restore(self) -> None:
        """Takes the list and the tolerance of the project that was read."""
        project = not_none(QgsProject.instance())
        text, found = project.readEntry(SECTION, 'rules', '')
        tolerance, _ = project.readDoubleEntry(SECTION, 'tolerance', -1.0)
        with self.quiet():
            for name in OPTIONS:
                checked, _ = project.readBoolEntry(SECTION, name, False)
                getattr(self.dock, name).setChecked(checked)
            self.dock.tolerance.setValue(
                tolerance if found else self.dock.tolerance.minimum()
            )
            ignored, _ = project.readEntry(SECTION, 'ignored', '')
            self.dock.results.ignored.load(ignored)
            self.dock.results.refresh()
            self.pending = json.loads(text) if found and text else None
            if self.pending is None:
                self.dock.rule_list.clear()
            self.fill()

    def fill(self) -> None:
        """Rebuilds the list from what the project had, once the rules are
        known."""
        known = {rule.name: rule for rule in self.dock.form.rules}
        # Nothing to take back means that the list is the user's, and stays
        if self.pending is None or not known:
            return
        with self.quiet():
            self.dock.rule_list.replace(
                [
                    Entry(
                        known[item['rule']],
                        item['layer'],
                        item['layer_name'],
                        item['unit'],
                        item['other'],
                        item['other_name'],
                        None,
                        bool(item['checked']),
                    )
                    for item in self.pending
                    if item['rule'] in known
                ]
            )
        self.pending = None
