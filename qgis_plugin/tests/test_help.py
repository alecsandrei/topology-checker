"""The help of the rule algorithms in the Processing toolbox."""

from __future__ import annotations

import pytest

from topology_rules.algorithms.rule import RuleAlgorithm
from topology_rules.core import binary
from topology_rules.provider import Provider

HELP_URL = 'https://alecsandrei.github.io/topology-checker/stable/rules/'


@pytest.fixture
def rule_algorithms(executable: str, provider: Provider) -> list[RuleAlgorithm]:
    algorithms = [
        algorithm
        for algorithm in provider.algorithms()
        if isinstance(algorithm, RuleAlgorithm)
    ]
    assert len(algorithms) == len(binary.rules())
    return algorithms


def needs(rule: binary.Rule) -> str:
    """What the rule takes, as the help says it."""
    if rule.other is None:
        return f'one {rule.layer} layer'
    return f'a {rule.layer} layer and a {rule.other} layer'


def test_every_rule_links_to_the_page_of_the_rules(
    rule_algorithms: list[RuleAlgorithm],
):
    for algorithm in rule_algorithms:
        assert algorithm.helpUrl() == HELP_URL, algorithm.name()


def test_every_rule_help_has_the_description(
    rule_algorithms: list[RuleAlgorithm],
):
    for algorithm in rule_algorithms:
        assert algorithm.rule.description in algorithm.shortHelpString()


def test_every_rule_help_says_which_layers_it_needs(
    rule_algorithms: list[RuleAlgorithm],
):
    for algorithm in rule_algorithms:
        if algorithm.rule.layer == 'any':
            continue
        assert needs(algorithm.rule) in algorithm.shortHelpString(), (
            algorithm.name()
        )


def test_every_rule_help_says_what_the_tolerance_is(
    rule_algorithms: list[RuleAlgorithm],
):
    for algorithm in rule_algorithms:
        assert 'tolerance' in algorithm.shortHelpString().lower()


def test_a_two_layer_rule_names_both_layers(
    rule_algorithms: list[RuleAlgorithm],
):
    covered = next(
        a
        for a in rule_algorithms
        if a.name() == 'point_must_be_covered_by_lines'
    )
    assert 'a point layer and a line layer' in covered.shortHelpString()


def test_a_one_layer_rule_names_its_layer(
    rule_algorithms: list[RuleAlgorithm],
):
    dangles = next(
        a for a in rule_algorithms if a.name() == 'line_must_not_have_dangles'
    )
    assert 'one line layer' in dangles.shortHelpString()
