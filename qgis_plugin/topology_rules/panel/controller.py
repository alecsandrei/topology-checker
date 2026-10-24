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
from qgis.PyQt.QtCore import Qt

from .._typing import Parameters
from ..algorithms.base import OUTPUT, TOLERANCE
from ..algorithms.check import COUNTS, INVALID, LAYERS, RULES, SCOPE
from ..core.progress import Stage
from ..core.style import ONE, ColourBy, Legend, entry_legend
from ..enums import PROVIDER_ID
from ..utils import not_none, tr
from .entries import Entry, project_layer
from .groups import add_errors, remove_errors
from .icons import icon
from .messages import Done, RunFeedback, Staged, push_failure, push_summary
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
    # The errors of each rule that is done, by its place in the run
    done: dict[int, int] = dataclasses.field(default_factory=dict)
    # Whether the errors of the rules that are done are in the tree
    live: bool = False


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
    feedback = RunFeedback(names_of(todo))
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
    begin(dock, run)
    # In the order the run sends them, and before the task says it is done
    queued = Qt.ConnectionType.QueuedConnection
    t.cast(t.Any, feedback.stage_changed).connect(
        lambda staged: stage_changed(dock, run, staged), queued
    )
    t.cast(t.Any, feedback.rule_done).connect(
        lambda found: rule_done(dock, run, found), queued
    )
    task.executed.connect(lambda ok, results: done(dock, run, ok, results))
    task.taskTerminated.connect(lambda: LIVE.discard(run))
    dock.rules_page.setEnabled(False)
    dock.progress.show()
    dock.stage.show()
    dock.update_buttons()
    not_none(QgsApplication.taskManager()).addTask(task)


def names_of(todo: list[Entry]) -> dict[str, str]:
    """The names of the layers of the rules, by their ids, as the errors
    read from the error layers have them."""
    names: dict[str, str] = {}
    for entry in todo:
        for layer_id in (entry.layer, entry.other):
            layer = None if layer_id is None else project_layer(layer_id)
            if layer is not None:
                names[layer.id()] = layer.name()
    return names


def begin(dock: Dock, run: Run) -> None:
    """Gives the panel to a run that starts: the errors of the last run are
    gone, and the errors of this one come as its rules are done."""
    remove_errors()
    dock.legends = {}
    dock.run_keys = {}
    dock.results.begin(rule_labels(run.entries), run.crs)
    dock.set_error_count(0)
    dock.progress.setRange(0, 0)
    dock.stage.say(tr('Reading layers…'))


def current(dock: Dock, run: Run) -> bool:
    """Whether the panel is there and waits for this run."""
    return not sip.isdeleted(dock) and dock.run is run


def stage_changed(dock: Dock, run: Run, staged: Staged) -> None:
    if not current(dock, run):
        return
    stage, index, total = staged
    if stage is Stage.READING:
        if run.done:
            # The run starts over, without the features that stopped it
            run.done.clear()
            run.live = False
            for entry in run.entries:
                entry.errors = None
            dock.rule_list.show_entries()
            begin(dock, run)
        dock.stage.say(tr('Reading layers…'))
        return
    if total != len(run.entries):
        return
    if stage is Stage.WRITING:
        dock.stage.say(tr('Writing errors…'))
    elif index is None:
        dock.stage.say(
            tr('Checking 1 rule…')
            if total == 1
            else tr('Checking {count} rules…').format(count=total)
        )
    elif 0 <= index < total:
        template = (
            tr('Checking {rule}…')
            if total == 1
            else tr('Checking {rule} ({index} of {total})…').format(
                rule='{rule}', index=index + 1, total=total
            )
        )
        dock.stage.say_about(template, rule_label(run.entries[index]))
    if index is not None or run.done:
        dock.progress.setRange(0, total)
        dock.progress.setValue(len(run.done))


def rule_done(dock: Dock, run: Run, found: Done) -> None:
    """Shows what a rule that is done found: its count in the list and, when
    the engine has them, its errors in the tree."""
    if not current(dock, run):
        return
    done, rows = found
    if done.total != len(run.entries) or not 0 <= done.index < done.total:
        return
    entry = run.entries[done.index]
    key = f'e{done.index}'
    run.done[done.index] = done.count
    dock.run_keys[entry.key] = key
    if rows is not None:
        run.live = True
        dock.results.add_rule(key, rows, done.layers)
        entry.errors = len(dock.results.active(key))
    else:
        entry.errors = done.count
        if not run.live:
            dock.results.waiting()
    dock.rule_list.set_count(entry)
    dock.set_error_count(
        len(dock.results.active()) if run.live else sum(run.done.values())
    )
    dock.progress.setRange(0, done.total)
    dock.progress.setValue(len(run.done))


def settle(dock: Dock) -> None:
    """Lets go of the run and gives the panel back."""
    dock.run = dock.task = dock.feedback = dock.context = None
    dock.rules_page.setEnabled(True)
    dock.progress.hide()
    dock.progress.setRange(0, 0)
    dock.stage.hide()
    dock.results.end()
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
    """Shows the outcome of a run when its task is done. A canceled run
    shows what the rules that were done found, as a run of those rules
    would. A run that failed keeps what it showed of them."""
    feedback = run.feedback
    todo = run.entries
    settle(dock)
    if not run.live:
        # The tab counted errors that the tree does not have: the engine
        # gives them only with the layers
        dock.set_error_count(0)
    bar = not_none(dock.iface.messageBar())
    canceled = feedback.isCanceled()
    if not ok and not canceled:
        if feedback.errors:
            push_failure(bar, feedback.errors[-1], feedback.textLog())
        return
    if canceled and OUTPUT not in results:
        # Nothing was written: what the list and the tree show is all
        push_summary(
            bar,
            canceled_text(
                len(run.done),
                len(todo),
                len(dock.results.active())
                if run.live
                else sum(run.done.values()),
            ),
            Qgis.MessageLevel.Info,
        )
        return
    counts = loads(results.get(COUNTS), [])
    for entry, count in zip(todo, counts, strict=False):
        entry.errors = int(count)
    dock.rule_list.show_entries()
    show_errors(
        dock,
        run,
        Path(str(results[OUTPUT])),
        results,
        counts,
        switch=not canceled,
    )
    recount(dock)
    total = len(dock.results.active())
    ignored = len(dock.results.rows) - total
    skipped = int(results.get(INVALID, 0))
    show = (tr('Show'), dock.show_invalid) if skipped else None
    if canceled:
        push_summary(
            bar,
            canceled_text(len(counts), len(todo), total),
            Qgis.MessageLevel.Info,
            show,
        )
        return
    text = summary(total, len(todo), skipped, dock.unselected, ignored)
    level = Qgis.MessageLevel.Info if total else Qgis.MessageLevel.Success
    push_summary(bar, text, level, show)


def canceled_text(done: int, total: int, errors: int) -> str:
    """How far a canceled run went: 'Canceled: 3 of 8 rules checked, 120
    errors.'"""
    if not done:
        return tr('Canceled.')
    if errors == 1:
        return tr('Canceled: {done} of {total} rules checked, 1 error.').format(
            done=done, total=total
        )
    return tr(
        'Canceled: {done} of {total} rules checked, {errors} errors.'
    ).format(done=done, total=total, errors=errors)


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
    """Gives each rule of the list that was checked the number of its errors
    that count."""
    for entry in dock.entries:
        key = dock.run_keys.get(entry.key)
        if key is not None:
            entry.errors = len(dock.results.active(key))
            dock.rule_list.set_count(entry)


def show_errors(
    dock: Dock,
    run: Run,
    path: Path,
    results: dict[str, t.Any],
    counts: list[int],
    switch: bool = True,
) -> None:
    """Replaces the errors of the last run with those of this one, which
    take the place of what the tree showed during the run."""
    todo = run.entries
    if not run.live:
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
    dock.legends = legends_of(todo, counts)
    legend = dock.legends[ColourBy.RULE_AND_LAYER]
    grouping, labels = legend.grouping, dict(legend.labels)
    dock.run_keys = {
        entry.key: f'e{i}' for i, entry in enumerate(todo[: len(counts)])
    }
    rows = read_rows(add_errors(path, dock.legends, run.crs), names, grouping)
    if run.live:
        dock.results.hand_over(rows, layers, grouping, labels)
    else:
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
    if active and switch:
        dock.tabs.setCurrentIndex(ERRORS_TAB)


def rule_label(entry: Entry) -> str:
    """The name of the group of the errors of a rule of a run."""
    return f'{entry.layer_name}: {entry.title}'


def rule_labels(todo: list[Entry]) -> dict[str, str]:
    return {f'e{i}': rule_label(entry) for i, entry in enumerate(todo)}


def legends_of(todo: list[Entry], counts: list[int]) -> dict[ColourBy, Legend]:
    """The groups of the errors of a run for each way of colouring them: a
    rule of the list, a rule on any layer, a layer, or all."""
    groups = {
        ColourBy.RULE_AND_LAYER: list(rule_labels(todo).items()),
        ColourBy.RULE: [
            (entry.rule.name, entry.rule.display_name) for entry in todo
        ],
        ColourBy.LAYER: [(entry.layer, entry.layer_name) for entry in todo],
        ColourBy.ONE: [(ONE, tr('Errors')) for _ in todo],
    }
    return {
        colour: entry_legend(counts, pairs) for colour, pairs in groups.items()
    }


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
