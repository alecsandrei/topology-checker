"""The command lines and the definition file that the plugin builds."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
import tomllib
from qgis.core import QgsProcessingFeedback

from topology_rules.core.binary import Rule
from topology_rules.core.commands import (
    Check,
    definition_command,
    rule_counts,
    utility_command,
    validate_command,
    write_config,
)
from topology_rules.core.layers import Dataset
from topology_rules.core.runner import Completed, error_message
from topology_rules.exceptions import RunError

DANGLES = Rule('line/must-not-have-dangles', 'x', 'line', None)
COVERED = Rule('point/must-be-covered-by-lines', 'x', 'point', 'line')


def test_validate_command():
    assert validate_command('/bin/tc', 'c.toml', None) == [
        '/bin/tc',
        '--json',
        'validate',
        'c.toml',
        '--stream',
    ]
    assert validate_command('/bin/tc', 'c.toml', 0.5)[5:7] == [
        '--tolerance',
        '0.5',
    ]
    assert validate_command('/bin/tc', 'c.toml', None, stream=False) == [
        '/bin/tc',
        '--json',
        'validate',
        'c.toml',
    ]


def test_definition_command_writes_the_errors():
    assert definition_command('/bin/tc', 'c.toml', 0.5, 'o.gpkg') == [
        '/bin/tc',
        '--json',
        'validate',
        'c.toml',
        '--summary',
        '--tolerance',
        '0.5',
        '-o',
        'o.gpkg',
        '--overwrite',
    ]


def test_config_lists_each_dataset_once(tmp_path: Path):
    roads = Dataset('/d/city.gpkg', 'roads')
    stops = Dataset('/d/stops.geojson')
    path = tmp_path / 'checks.toml'
    aliases = write_config(
        path,
        [
            Check(DANGLES, roads),
            Check(COVERED, stops, roads),
        ],
    )
    assert aliases == {roads: 'roads', stops: 'stops'}
    config = tomllib.loads(path.read_text(encoding='utf-8'))
    assert config['layers'] == {
        'roads': {'path': '/d/city.gpkg', 'layer': 'roads'},
        'stops': {'path': '/d/stops.geojson'},
    }
    assert config['rules'] == [
        {'rule': 'line/must-not-have-dangles', 'layer': 'roads'},
        {
            'rule': 'point/must-be-covered-by-lines',
            'layer': 'stops',
            'other': 'roads',
        },
    ]


def test_config_makes_aliases_unique_and_escapes_text(tmp_path: Path):
    first = Dataset('/a/roads.gpkg', 'roads')
    second = Dataset('/b/roads.gpkg', 'roads')
    odd = Dataset('C:\\data\\"quoted" ü.gpkg', 'a "b"')
    path = tmp_path / 'checks.toml'
    aliases = write_config(
        path,
        [Check(DANGLES, first), Check(DANGLES, second), Check(DANGLES, odd)],
    )
    assert list(aliases.values()) == ['roads', 'roads_2', 'a "b"']
    config = tomllib.loads(path.read_text(encoding='utf-8'))
    assert list(config['layers']) == ['roads', 'roads_2', 'a "b"']
    assert config['layers']['a "b"']['path'] == 'C:\\data\\"quoted" ü.gpkg'
    assert [check['layer'] for check in config['rules']] == [
        'roads',
        'roads_2',
        'a "b"',
    ]


def test_an_exported_dataset_is_named_after_its_layer(tmp_path: Path):
    exported = Dataset('/tmp/work/my_lines.gpkg', 'my_lines')
    aliases = write_config(tmp_path / 'checks.toml', [Check(DANGLES, exported)])
    assert aliases == {exported: 'my_lines'}


def test_a_dataset_without_a_layer_name_is_named_after_its_file(
    tmp_path: Path,
):
    source = Dataset('/d/roads.geojson')
    aliases = write_config(tmp_path / 'checks.toml', [Check(DANGLES, source)])
    assert aliases == {source: 'roads'}


def test_the_same_dataset_in_two_checks_has_one_alias(tmp_path: Path):
    roads = Dataset('/d/city.gpkg', 'roads')
    aliases = write_config(
        tmp_path / 'checks.toml',
        [Check(DANGLES, roads), Check(COVERED, roads, roads)],
    )
    assert aliases == {roads: 'roads'}


def test_utility_command():
    assert utility_command(
        '/bin/tc', 'merge-lines', Dataset('a.gpkg', 'a'), 'o.gpkg'
    ) == [
        '/bin/tc',
        '--overwrite',
        'utilities',
        'merge-lines',
        'a.gpkg',
        '--layer',
        'a',
        '-o',
        'o.gpkg',
    ]


def report(*counts: dict[str, int]) -> str:
    return json.dumps(
        {
            'tolerance': 0.0,
            'warnings': [],
            'rules': [
                {'counts': count, 'layer': 'a', 'other': None}
                for count in counts
            ],
        }
    )


def test_counts_errors_for_valid_and_invalid_data():
    feedback = QgsProcessingFeedback()
    assert rule_counts(Completed(0, report({}), ''), feedback) == [0]
    assert rule_counts(Completed(1, report({'dangle': 2}), ''), feedback) == [2]


def test_counts_errors_of_each_rule():
    completed = Completed(1, report({'dangle': 2, 'gap': 1}, {}, {'x': 4}), '')
    assert rule_counts(completed, QgsProcessingFeedback()) == [3, 0, 4]


def test_exit_code_two_raises_the_json_error():
    completed = Completed(2, '', 'warning: x\n{"error": "no such layer"}\n')
    with pytest.raises(RunError, match='no such layer'):
        rule_counts(completed, QgsProcessingFeedback())


def test_an_unreadable_report_raises_with_its_start_in_the_log():
    """The message is short; the log has the start of the report."""
    feedback = QgsProcessingFeedback()
    text = '{"tolerance": 0, "rules": ' + 'x' * 1000
    with pytest.raises(RunError) as raised:
        rule_counts(Completed(0, text, ''), feedback)
    assert str(raised.value) == "Could not read the executable's report."
    log = feedback.textLog()
    assert text[:200] + '…' in log
    assert 'x' * 300 not in log


def test_error_message_falls_back_to_text_and_code():
    assert error_message(Completed(2, '', 'plain failure')) == 'plain failure'
    assert error_message(Completed(3, '', '')) == 'Exited with code 3'
