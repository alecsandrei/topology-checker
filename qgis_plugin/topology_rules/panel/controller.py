from __future__ import annotations

import dataclasses
import json
import typing as t
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsApplication,
    QgsCoordinateReferenceSystem,
    QgsProcessingAlgRunnerTask,
    QgsProcessingContext,
    QgsProcessingUtils,
    QgsProject,
)
from qgis.PyQt import sip

from .._typing import Parameters
from ..algorithms.base import OUTPUT, TOLERANCE
from ..algorithms.check import COUNTS, INVALID, LAYERS, RULES, SCOPE
from ..core.style import entry_grouping
from ..enums import PROVIDER_ID
from ..utils import not_none, tr
from .entries import Entry, project_layer
from .groups import add_errors, remove_errors
from .icons import icon
from .messages import RunFeedback, push_failure, push_summary
from .rows import read_rows

if t.TYPE_CHECKING:
    from .dock import Dock

ERRORS_TAB = 1


@dataclasses.dataclass(eq=False)
class Run:
    """A run in progress: its task, what the task works with, the rules it
    was started with, and the CRS of its errors, which is that of the first
    layer."""

    task: QgsProcessingAlgRunnerTask
    context: QgsProcessingContext
    feedback: RunFeedback
    entries: list[Entry]
    crs: QgsCoordinateReferenceSystem


# A task works with its context and feedback in another thread, so they stay
# alive until the task has reported, whatever becomes of the panel
LIVE: set[Run] = set()


def start(dock: Dock) -> None:
    """Runs the checked rules of the list in a task."""
    todo = dock.runnable()
    if dock.task is not None or not todo:
        return
    gone = [entry.title for entry in todo if entry.missing]
    if gone:
        dock.rule_list.show_entries()
        dock.warn(
            tr('Layers are missing for: {rules}').format(rules=', '.join(gone))
        )
        return
    algorithm = not_none(
        QgsApplication.processingRegistry()
    ).createAlgorithmById(f'{PROVIDER_ID}:check_rules')
    if algorithm is None:
        not_none(dock.iface.messageBar()).pushCritical(
            tr('Topology Rules'), tr('The rules cannot be run.')
        )
        return
    scope, dock.unselected = scope_of(dock, todo)
    dock.runs += 1
    for entry in dock.entries:
        entry.errors = None
    dock.rule_list.invalid = {}
    dock.rule_list.show_entries()
    context = QgsProcessingContext()
    context.setProject(not_none(QgsProject.instance()))
    feedback = RunFeedback()
    task = QgsProcessingAlgRunnerTask(
        algorithm,
        parameters(todo, dock.tolerance_value(), dock.runs, scope),
        context,
        feedback,
    )
    first = project_layer(todo[0].layer)
    crs = QgsCoordinateReferenceSystem() if first is None else first.crs()
    run = Run(task, context, feedback, list(todo), crs)
    LIVE.add(run)
    dock.run, dock.task = run, task
    dock.context, dock.feedback = context, feedback
    task.executed.connect(lambda ok, results: done(dock, run, ok, results))
    task.taskTerminated.connect(lambda: LIVE.discard(run))
    dock.rules_page.setEnabled(False)
    dock.progress.show()
    dock.update_buttons()
    not_none(QgsApplication.taskManager()).addTask(task)


def settle(dock: Dock) -> None:
    """Lets go of the run and gives the panel back."""
    dock.run = dock.task = dock.feedback = dock.context = None
    dock.rules_page.setEnabled(True)
    dock.progress.hide()
    dock.update_buttons()


def abandon(dock: Dock) -> None:
    """Stops the run in progress and drops what it finds: the project it was
    for is gone, or the panel is."""
    run = dock.run
    if run is None:
        return
    run.feedback.cancel()
    settle(dock)


def done(dock: Dock, run: Run, ok: bool, results: dict[str, t.Any]) -> None:
    """The task of a run ended. What it found is for the panel that started
    it, if the panel is there and still waits for this run."""
    LIVE.discard(run)
    # The panel can be gone when the plugin was unloaded during a run
    if sip.isdeleted(dock) or dock.run is not run:
        return
    finish(dock, run, ok, results)


def finish(dock: Dock, run: Run, ok: bool, results: dict[str, t.Any]) -> None:
    """Shows the outcome of a run when its task is done."""
    feedback = run.feedback
    todo = run.entries
    settle(dock)
    bar = not_none(dock.iface.messageBar())
    if feedback.isCanceled():
        return
    if not ok:
        if feedback.errors:
            push_failure(bar, feedback.errors[-1], feedback.textLog())
        return
    counts = loads(results.get(COUNTS), [])
    for entry, count in zip(todo, counts, strict=False):
        entry.errors = int(count)
    dock.rule_list.show_entries()
    show_errors(
        dock, todo, Path(str(results[OUTPUT])), results, counts, run.crs
    )
    recount(dock)
    total = len(dock.results.active())
    ignored = len(dock.results.rows) - total
    skipped = int(results.get(INVALID, 0))
    text = summary(total, len(todo), skipped, dock.unselected, ignored)
    level = Qgis.MessageLevel.Info if total else Qgis.MessageLevel.Success
    show = (tr('Show'), dock.show_invalid) if skipped else None
    push_summary(bar, text, level, show)


def show_run_button(dock: Dock, count: int) -> None:
    """Run with the number of rules to run, or Cancel while a run is on."""
    button = dock.run_button
    if dock.task is not None:
        button.setText(tr('Cancel'))
        button.setIcon(icon('stop'))
        button.setToolTip(tr('Stop the run'))
        button.setEnabled(True)
        return
    button.setText(
        tr('Run')
        if count == 0
        else tr('Run 1 rule')
        if count == 1
        else tr('Run {count} rules').format(count=count)
    )
    button.setIcon(icon('run'))
    button.setToolTip(tr('Check the layers against the rules'))
    button.setEnabled(dock.available and count > 0)


def recount(dock: Dock) -> None:
    """Gives each rule of the list the number of its errors that count."""
    for entry in dock.entries:
        key = dock.run_keys.get(entry.key)
        if key is not None:
            entry.errors = len(dock.results.active(key))
    dock.rule_list.show_entries()


def show_errors(
    dock: Dock,
    todo: list[Entry],
    path: Path,
    results: dict[str, t.Any],
    counts: list[int],
    crs: QgsCoordinateReferenceSystem,
) -> None:
    """Replaces the errors of the last run with those of this one."""
    dock.results.clear()
    remove_errors()
    layers = loads(results.get(LAYERS), {})
    names = {
        alias: layer.name()
        for alias, layer_id in layers.items()
        if (layer := project_layer(layer_id)) is not None
    }
    # The errors of each rule of the list are a group, in the order of the
    # list: the tool numbers them rule after rule
    grouping = entry_grouping(counts)
    labels: dict[str, str] = {}
    dock.run_keys = {}
    for index, entry in enumerate(todo):
        labels[f'e{index}'] = f'{entry.layer_name}: {entry.title}'
        dock.run_keys[entry.key] = f'e{index}'
    rows = read_rows(
        add_errors(path, grouping, labels, list(labels), crs), names, grouping
    )
    dock.results.show_rows(rows, layers, grouping, labels)
    # A layer can be several inputs of the run, each with a name of its own
    invalid: dict[str, int] = {}
    for alias, layer_id in layers.items():
        invalid[layer_id] = invalid.get(layer_id, 0) + sum(
            1 for r in rows if not r.rule_name and r.layer_alias == alias
        )
    dock.rule_list.set_invalid(invalid)
    active = dock.results.active()
    dock.set_error_count(len(active))
    if active:
        dock.tabs.setCurrentIndex(ERRORS_TAB)


def scope_of(
    dock: Dock, todo: list[Entry]
) -> tuple[dict[str, t.Any], list[str]]:
    """The part of the layers to check as the options say now, and the
    layers that have no selected features when only those are checked."""
    scope: dict[str, t.Any] = {}
    unselected: list[str] = []
    if dock.selected_only.isChecked():
        chosen: dict[str, list[int]] = {}
        for entry in todo:
            # The layer a rule checks, not the one it checks it against
            layer = project_layer(entry.layer)
            if layer is None or entry.layer in chosen:
                continue
            chosen[entry.layer] = layer.selectedFeatureIds()
            if not chosen[entry.layer]:
                unselected.append(layer.name())
        scope['selected'] = chosen
    if dock.extent_only.isChecked():
        canvas = not_none(dock.iface.mapCanvas())
        box = canvas.extent()
        scope['extent'] = {
            'rect': [
                box.xMinimum(),
                box.yMinimum(),
                box.xMaximum(),
                box.yMaximum(),
            ],
            'crs': canvas.mapSettings().destinationCrs().toWkt(),
        }
    return scope, unselected


def parameters(
    entries: list[Entry],
    tolerance: float | None,
    number: int,
    scope: dict[str, t.Any],
) -> Parameters:
    """The parameters of the algorithm that checks the rules of the list."""
    found: Parameters = {
        RULES: json.dumps(
            [
                {
                    'rule': entry.rule.name,
                    'layer': entry.layer,
                    'other': entry.other,
                }
                for entry in entries
            ]
        ),
        # In the Processing temp folder, which outlives the error layers
        OUTPUT: QgsProcessingUtils.generateTempFilename(
            f'topology_errors_{number}.gpkg'
        ),
    }
    if tolerance is not None:
        found[TOLERANCE] = tolerance
    if scope:
        found[SCOPE] = json.dumps(scope)
    return found


def loads(text: object, default: t.Any) -> t.Any:
    try:
        return json.loads(str(text))
    except ValueError:
        return default


def summary(
    errors: int,
    rules: int,
    skipped: int,
    unselected: list[str],
    ignored: int = 0,
) -> str:
    """How a run went, such as '12 errors in 3 rules'."""
    if errors == 0:
        text = tr('No errors found.')
    else:
        text = (
            tr('{errors} errors in {rules} rules').format(
                errors=errors, rules=rules
            )
            if rules != 1
            else tr('{errors} errors in 1 rule').format(errors=errors)
        )
    if ignored:
        text += ' · ' + tr('{count} ignored').format(count=ignored)
    if skipped:
        text += ' · ' + tr('{count} invalid geometries skipped').format(
            count=skipped
        )
    if unselected:
        text += ' · ' + tr('No selected features in {layers}').format(
            layers=', '.join(unselected)
        )
    return text
