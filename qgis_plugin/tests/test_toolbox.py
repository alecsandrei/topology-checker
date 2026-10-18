"""What the Processing toolbox shows of the algorithms: search tags, icons,
help and names."""

from __future__ import annotations

import pytest
from qgis.core import QgsApplication, QgsProcessingAlgorithm

from topology_rules.algorithms.rule import RuleAlgorithm
from topology_rules.core import binary
from topology_rules.provider import Provider

GROUP_WORDS = {'point': 'points', 'line': 'lines', 'polygon': 'polygons'}
# Words of a rule name that say nothing about what it checks
FILLER = {'must', 'not', 'be', 'have', 'by', 'with', 'of'}


@pytest.fixture
def algorithms(
    executable: str, provider: Provider
) -> list[QgsProcessingAlgorithm]:
    found = list(provider.algorithms())
    assert found
    return found


def visible(
    algorithms: list[QgsProcessingAlgorithm],
) -> list[QgsProcessingAlgorithm]:
    hidden = QgsProcessingAlgorithm.Flag.FlagHideFromToolbox
    return [a for a in algorithms if not a.flags() & hidden]


def rule_algorithms(
    algorithms: list[QgsProcessingAlgorithm],
) -> list[RuleAlgorithm]:
    found = [a for a in algorithms if isinstance(a, RuleAlgorithm)]
    assert len(found) == len(binary.rules())
    return found


def tags_of(algorithm: QgsProcessingAlgorithm) -> set[str]:
    return {tag.lower() for tag in algorithm.tags()}


def test_every_rule_is_found_by_searching_topology(
    algorithms: list[QgsProcessingAlgorithm],
):
    for algorithm in rule_algorithms(algorithms):
        assert 'topology' in tags_of(algorithm), algorithm.name()


def test_every_rule_has_the_word_of_its_geometry_as_a_tag(
    algorithms: list[QgsProcessingAlgorithm],
):
    for algorithm in rule_algorithms(algorithms):
        word = GROUP_WORDS.get(algorithm.rule.name.split('/')[0])
        if word is not None:
            assert word in tags_of(algorithm), algorithm.name()


def test_every_rule_has_the_words_of_its_name_as_tags(
    algorithms: list[QgsProcessingAlgorithm],
):
    for algorithm in rule_algorithms(algorithms):
        words = algorithm.rule.name.split('/')[1].split('-')
        content = {word for word in words if word not in FILLER}
        assert content <= tags_of(algorithm), algorithm.name()


def test_the_dangles_rule_is_found_by_searching_dangles(
    algorithms: list[QgsProcessingAlgorithm],
):
    dangles = next(
        a for a in rule_algorithms(algorithms) if a.name().endswith('dangles')
    )
    assert {'topology', 'lines', 'dangles'} <= tags_of(dangles)


def test_every_algorithm_has_an_icon_of_the_plugin(
    algorithms: list[QgsProcessingAlgorithm],
):
    generic = (
        QgsApplication.getThemeIcon('/processingAlgorithm.svg')
        .pixmap(24, 24)
        .toImage()
    )
    for algorithm in algorithms:
        icon = algorithm.icon()
        assert not icon.isNull(), algorithm.name()
        assert icon.pixmap(24, 24).toImage() != generic, algorithm.name()


def test_every_algorithm_has_a_link_to_its_help(
    algorithms: list[QgsProcessingAlgorithm],
):
    for algorithm in visible(algorithms):
        assert algorithm.helpUrl().startswith('https://'), algorithm.name()


def test_every_algorithm_has_a_help_text(
    algorithms: list[QgsProcessingAlgorithm],
):
    for algorithm in visible(algorithms):
        assert algorithm.shortHelpString().strip(), algorithm.name()


def test_no_two_algorithms_have_the_same_name(
    algorithms: list[QgsProcessingAlgorithm],
):
    names = [algorithm.name() for algorithm in algorithms]
    assert len(names) == len(set(names))


def test_no_two_algorithms_have_the_same_display_name(
    algorithms: list[QgsProcessingAlgorithm],
):
    names = [algorithm.displayName() for algorithm in visible(algorithms)]
    assert len(names) == len(set(names))


def test_every_algorithm_is_in_a_group(
    algorithms: list[QgsProcessingAlgorithm],
):
    for algorithm in visible(algorithms):
        assert algorithm.group().strip(), algorithm.name()
        assert algorithm.groupId().strip(), algorithm.name()
