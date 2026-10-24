"""The texts of runs, exports and Processing that a user reads are
translated, and the values in them are put in after the translation."""

from __future__ import annotations

import importlib
import subprocess
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsGeometry, QgsProcessingFeedback, QgsProject

from topology_rules.algorithms.check import CheckRulesAlgorithm
from topology_rules.algorithms.definition import CheckDefinition
from topology_rules.algorithms.rule import RuleAlgorithm
from topology_rules.algorithms.sinks import loaded_name, sink_label
from topology_rules.algorithms.utilities import (
    ExplodeLines,
    MergeLines,
    report_skipped,
)
from topology_rules.core import runner, toml
from topology_rules.core.binary import Rule
from topology_rules.core.engine import Package, describe
from topology_rules.core.inputs import Skipped
from topology_rules.core.layers import Input, require_kind
from topology_rules.core.runner import Completed, error_message
from topology_rules.exceptions import RunError
from topology_rules.panel.entries import Entry
from topology_rules.panel.exchange import DefinitionError, export_definition
from topology_rules.panel.groups import group_name

from .helpers import line_layer, point_layer

DANGLES = Rule('line/must-not-have-dangles', 'Finds dangles.', 'line', None)
COVERED = Rule(
    'point/must-be-covered-by-lines', 'Finds points.', 'point', 'line'
)

pytestmark = pytest.mark.usefixtures('marked')


class Hanging:
    """A process that never answers."""

    pid = 0
    returncode = None

    def communicate(self, timeout: float | None = None) -> tuple[str, str]:
        raise subprocess.TimeoutExpired('tc', timeout or 0)


class Feedback(QgsProcessingFeedback):
    """Keeps what is said in the log."""

    def __init__(self) -> None:
        super().__init__()
        self.said: list[str] = []

    def pushInfo(self, info: str | None) -> None:
        self.said.append(info or '')

    def pushWarning(self, warning: str | None) -> None:
        self.said.append(warning or '')


# A run of the executable


def test_a_failed_run_without_a_message_says_its_exit_code():
    assert error_message(Completed(3, '', '')) == '<Exited with code 3>'


def test_a_program_that_cannot_start_says_so(tmp_path: Path):
    missing = str(tmp_path / 'nowhere')
    with pytest.raises(RunError) as raised:
        runner.run([missing])
    assert str(raised.value).startswith(f'<Could not run {missing}: ')
    assert str(raised.value).endswith('>')


def test_a_program_that_does_not_answer_says_how_long_it_had(
    monkeypatch: pytest.MonkeyPatch,
):
    monkeypatch.setattr(subprocess, 'Popen', lambda *a, **k: Hanging())
    monkeypatch.setattr(runner, 'stop', lambda process: None)
    with pytest.raises(RunError) as raised:
        runner.run(['tc'], timeout=-1)
    assert str(raised.value) == '<tc did not answer within -1 s>'


def test_a_canceled_run_says_so(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(subprocess, 'Popen', lambda *a, **k: Hanging())
    monkeypatch.setattr(runner, 'stop', lambda process: None)
    feedback = QgsProcessingFeedback()
    feedback.cancel()
    with pytest.raises(RunError) as raised:
        runner.run(['tc'], feedback)
    assert str(raised.value) == '<Canceled>'


def test_the_log_names_the_engine():
    package = Package('0.2.1', Path('/site'))
    assert describe(package) == (
        f'<Engine: Python package topology-checker 0.2.1 ({package.location})>'
    )


def test_a_layer_of_another_geometry_says_what_is_needed():
    lines = line_layer()
    with pytest.raises(RunError) as raised:
        require_kind(Input(lines.name(), lines, lines), 'point')
    assert str(raised.value) == (
        '<lines is <a line layer>, but <a point layer> is needed.>'
    )


# A topology.toml


def test_reading_a_definition_on_an_old_python_says_so(
    monkeypatch: pytest.MonkeyPatch,
):
    def missing(name: str) -> t.NoReturn:
        raise ImportError(name)

    monkeypatch.setattr(importlib, 'import_module', missing)
    with pytest.raises(toml.TomlUnavailable) as raised:
        toml.parser()
    assert str(raised.value) == (
        '<Reading topology.toml needs Python 3.11 or newer. '
        'The Python of this QGIS is older.>'
    )


def test_an_export_without_checked_rules_says_so(tmp_path: Path):
    entry = Entry(DANGLES, 'a', 'roads', 'm', checked=False)
    with pytest.raises(DefinitionError) as raised:
        export_definition([entry], None, tmp_path / 'topology.toml')
    assert str(raised.value) == '<Check at least one rule to export.>'


def test_an_export_names_the_layers_that_are_gone(tmp_path: Path):
    entries = [Entry(COVERED, 'gone', 'stops', 'm', 'also', 'roads')]
    with pytest.raises(DefinitionError) as raised:
        export_definition(entries, None, tmp_path / 'topology.toml')
    assert str(raised.value) == (
        '<These layers are not in the project: stops, roads>'
    )


def test_an_export_that_keeps_the_data_file_says_it_was_canceled(
    tmp_path: Path,
):
    lines = line_layer('LINESTRING (0 0, 1 1)')
    project = QgsProject.instance()
    assert project is not None
    project.addMapLayer(lines)
    path = tmp_path / 'topology.toml'
    (tmp_path / 'topology_data.gpkg').write_bytes(b'')
    entry = Entry(DANGLES, lines.id(), lines.name(), 'm')
    try:
        with pytest.raises(DefinitionError) as raised:
            export_definition([entry], None, path, overwrite=lambda _: False)
    finally:
        project.removeMapLayer(lines.id())
    assert str(raised.value) == '<Export canceled.>'
    assert not path.exists()


# Processing


def test_the_algorithms_have_translated_names_and_help():
    check, definition = CheckRulesAlgorithm(), CheckDefinition()
    merge, explode = MergeLines(), ExplodeLines()
    assert check.displayName() == '<Check rules>'
    assert definition.displayName() == '<Check topology definition>'
    assert merge.displayName() == '<Merge lines>'
    assert explode.displayName() == '<Explode lines>'
    for algorithm in (check, definition, merge, explode):
        assert algorithm.group() == '<Utilities>'
        assert algorithm.groupId() == 'utilities'
        assert algorithm.shortHelpString().startswith('<')


def test_a_rule_is_in_a_translated_group_and_says_what_it_takes():
    algorithm = RuleAlgorithm(COVERED)
    assert algorithm.group() == '<Points>'
    assert algorithm.groupId() == 'point'
    assert algorithm.shortHelpString().startswith(
        '<Finds points.\n\nTakes <a <point layer> and a <line layer>>. '
    )
    assert RuleAlgorithm(DANGLES).needs() == '<one <line layer>>'


def test_the_parameters_of_a_rule_are_translated():
    algorithm = RuleAlgorithm(COVERED)
    algorithm.initAlgorithm()
    found = {
        definition.name(): definition.description()
        for definition in algorithm.parameterDefinitions()
    }
    assert found['INPUT'] == '<Input layer>'
    assert found['OTHER'] == '<Other layer>'
    assert found['ERRORS_LINE'] == '<Line errors>'


def test_the_error_layers_have_translated_names():
    assert sink_label('polygon') == '<Polygon errors>'
    assert loaded_name('Lines must not have dangles', 'line') == (
        '<Lines must not have dangles (<line>)>'
    )


def test_the_features_left_out_of_a_utility_are_said_in_the_log():
    points = point_layer(name='stops')
    feedback = Feedback()
    skipped = [Skipped(4, QgsGeometry(), 'coordinates must be finite')]
    report_skipped(Input(points.name(), points, points), skipped, feedback)
    assert feedback.said == [
        '<1 features of stops were left out of the run>',
        '<Left out of the run: stops, feature 4: coordinates must be finite>',
    ]


# The panel


def test_the_group_of_the_error_layers_is_translated():
    assert group_name() == '<Topology errors>'
