"""Names that say which geometry a rule is about."""

from __future__ import annotations

import pytest
from qgis.core import QgsApplication, QgsProject

from topology_rules.core import binary
from topology_rules.core.binary import Rule
from topology_rules.enums import PROVIDER_ID
from topology_rules.panel.dock import Dock
from topology_rules.provider import Provider

from .helpers import add, layer_nodes, polygon_layer, rule_items

PLURALS = {
    'point': 'Points',
    'line': 'Lines',
    'polygon': 'Polygons',
    'geometry': 'Geometries',
}


@pytest.mark.parametrize(
    ('name', 'display'),
    [
        ('line/must-not-have-dangles', 'Lines must not have dangles'),
        ('point/must-be-inside', 'Points must be inside'),
        ('geometry/must-not-be-multipart', 'Geometries must not be multipart'),
        ('polygon/must-not-overlap-with', 'Polygons must not overlap with'),
        ('point/must-be-covered-by-lines', 'Points must be covered by lines'),
    ],
)
def test_a_rule_name_starts_with_its_geometry(name: str, display: str):
    assert Rule(name, 'x', 'line', None).display_name == display


def test_every_rule_of_the_tool_starts_with_its_geometry(executable: str):
    for rule in binary.rules():
        group = rule.name.split('/')[0]
        assert rule.display_name.startswith(f'{PLURALS[group]} must'), rule.name


def test_no_two_rules_have_the_same_name(executable: str):
    names = [rule.display_name for rule in binary.rules()]
    assert len(names) == len(set(names))


@pytest.mark.parametrize(
    ('slug', 'display'),
    [
        ('line_must_not_have_dangles', 'Lines must not have dangles'),
        ('point_must_be_inside', 'Points must be inside'),
        ('geometry_must_not_be_multipart', 'Geometries must not be multipart'),
    ],
)
def test_processing_shows_the_same_names(
    executable: str, provider: Provider, slug: str, display: str
):
    algorithm = QgsApplication.processingRegistry().createAlgorithmById(
        f'{PROVIDER_ID}:{slug}'
    )
    assert algorithm is not None
    assert algorithm.displayName() == display
    assert algorithm.name() == slug


# In the dock the layer is on the screen, so the rule does not say it again


OVERLAPS = 'polygon/must-not-overlap'
OVERLAPS_WITH = 'polygon/must-not-overlap-with'


def polygons(name: str) -> object:
    layer = polygon_layer('POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))', name=name)
    QgsProject.instance().addMapLayer(layer)
    return layer


def test_the_rule_box_drops_the_geometry_word(dock: Dock):
    dock.input_box.setLayer(polygons('countries'))
    texts = {
        dock.rule_box.itemData(index).name: dock.rule_box.itemText(index)
        for index in range(dock.rule_box.count())
    }
    assert texts[OVERLAPS] == 'Must not overlap'
    assert texts[OVERLAPS_WITH] == 'Must not overlap with'
    assert texts['geometry/must-not-be-multipart'] == 'Must not be multipart'


def test_the_list_drops_the_geometry_word(dock: Dock):
    add(dock, OVERLAPS, polygons('countries'))
    assert [item.text(0) for item in rule_items(dock)] == ['Must not overlap']


def test_the_list_names_the_other_layer_after_the_rule(dock: Dock):
    add(dock, OVERLAPS_WITH, polygons('lakes'), polygons('countries'))
    assert [item.text(0) for item in rule_items(dock)] == [
        'Must not overlap with countries'
    ]
    assert layer_nodes(dock)[0].text(0) == 'lakes'


def test_the_tooltip_of_a_rule_keeps_the_geometry_word(dock: Dock):
    add(dock, OVERLAPS, polygons('countries'))
    (item,) = rule_items(dock)
    assert 'Polygons must not overlap' in item.toolTip(0)
    assert dock.entries[0].rule.description in item.toolTip(0)
