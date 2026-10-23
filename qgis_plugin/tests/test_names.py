"""Names that say which geometry a rule is about."""

from __future__ import annotations

import pytest
from qgis.core import QgsApplication, QgsProject

from topology_rules.core import binary
from topology_rules.core.binary import Rule
from topology_rules.enums import PROVIDER_ID
from topology_rules.panel.dock import Dock
from topology_rules.provider import Provider

from .helpers import (
    add,
    both_engines,
    click_run,
    layer_nodes,
    line_layer,
    node_text,
    point_layer,
    polygon_layer,
    result_nodes,
    rule_items,
)

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
        ('point/must-be-covered-by-lines', 'Points must be covered by lines'),
    ],
)
def test_a_rule_name_starts_with_its_geometry(name: str, display: str):
    assert Rule(name, 'x', 'line', None).display_name == display


# The rules that take a second layer of the same kind say so

WITH_ANOTHER = [
    ('point/must-not-overlap-with', 'Points must not overlap another layer'),
    ('line/must-not-overlap-with', 'Lines must not overlap another layer'),
    ('line/must-not-intersect-with', 'Lines must not intersect another layer'),
    (
        'line/must-not-intersect-or-touch-interior-with',
        'Lines must not intersect or touch interior of another layer',
    ),
    (
        'polygon/must-not-overlap-with',
        'Polygons must not overlap another layer',
    ),
]


@pytest.mark.parametrize(('name', 'display'), WITH_ANOTHER)
def test_a_rule_with_a_second_layer_says_another_layer(name: str, display: str):
    kind = name.partition('/')[0]
    assert Rule(name, 'x', kind, kind).display_name == display


@pytest.mark.parametrize(('name', 'display'), WITH_ANOTHER)
def test_the_short_name_of_such_a_rule_drops_the_geometry_word(
    name: str, display: str
):
    kind = name.partition('/')[0]
    short = display.partition(' ')[2]
    assert (
        Rule(name, 'x', kind, kind).short_name == short[:1].upper() + short[1:]
    )


def test_a_future_rule_with_a_second_layer_is_named_by_its_layers():
    rule = Rule('line/must-touch-with', 'x', 'line', 'line')
    assert rule.display_name == 'Lines must touch (two layers)'
    assert rule.short_name == 'Must touch (two layers)'


def test_no_rule_of_the_tool_is_named_with_a_bare_with(executable: str):
    for rule in binary.rules():
        assert not rule.display_name.endswith(' with'), rule.name
        assert not rule.short_name.endswith(' with'), rule.name


def test_the_rules_of_the_tool_with_a_second_layer_say_another_layer(
    executable: str,
):
    names = {rule.name: rule.display_name for rule in binary.rules()}
    for name, display in WITH_ANOTHER:
        assert names[name] == display


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
        ('line_must_not_overlap_with', 'Lines must not overlap another layer'),
        (
            'polygon_must_not_overlap_with',
            'Polygons must not overlap another layer',
        ),
        (
            'line_must_not_intersect_or_touch_interior_with',
            'Lines must not intersect or touch interior of another layer',
        ),
    ],
)
@both_engines
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


@both_engines
def test_the_rule_box_drops_the_geometry_word(dock: Dock):
    dock.input_box.setLayer(polygons('countries'))
    texts = {
        dock.rule_box.itemData(index).name: dock.rule_box.itemText(index)
        for index in range(dock.rule_box.count())
    }
    assert texts[OVERLAPS] == 'Must not overlap'
    assert texts[OVERLAPS_WITH] == 'Must not overlap another layer'
    assert texts['geometry/must-not-be-multipart'] == 'Must not be multipart'


@both_engines
def test_the_list_drops_the_geometry_word(dock: Dock):
    add(dock, OVERLAPS, polygons('countries'))
    assert [item.text(0) for item in rule_items(dock)] == ['Must not overlap']


@both_engines
def test_the_list_names_the_other_layer_after_the_rule(dock: Dock):
    add(dock, OVERLAPS_WITH, polygons('lakes'), polygons('countries'))
    assert [item.text(0) for item in rule_items(dock)] == [
        'Must not overlap with countries'
    ]
    assert layer_nodes(dock)[0].text(0) == 'lakes'


@both_engines
def test_the_tooltip_of_a_rule_keeps_the_geometry_word(dock: Dock):
    add(dock, OVERLAPS, polygons('countries'))
    (item,) = rule_items(dock)
    assert 'Polygons must not overlap' in item.toolTip(0)
    assert dock.entries[0].rule.description in item.toolTip(0)


@pytest.mark.parametrize(('name', 'display'), WITH_ANOTHER)
@both_engines
def test_the_rule_box_says_another_layer_for_a_second_layer_of_the_same_kind(
    dock: Dock, name: str, display: str
):
    kind = name.partition('/')[0]
    make = {'point': point_layer, 'line': line_layer, 'polygon': polygon_layer}
    layer = make[kind]()
    QgsProject.instance().addMapLayer(layer)
    dock.input_box.setLayer(layer)
    texts = {
        dock.rule_box.itemData(index).name: dock.rule_box.itemText(index)
        for index in range(dock.rule_box.count())
    }
    short = display.partition(' ')[2]
    assert texts[name] == short[:1].upper() + short[1:]


# The list and the results name the other layer, as they did


LABELS = [
    ('line/must-not-overlap-with', 'Must not overlap with countries'),
    ('line/must-not-intersect-with', 'Must not intersect with countries'),
    (
        'line/must-not-intersect-or-touch-interior-with',
        'Must not intersect or touch interior with countries',
    ),
]


@pytest.mark.parametrize(('name', 'label'), LABELS)
@both_engines
def test_the_list_keeps_with_and_the_other_layer(
    dock: Dock, name: str, label: str
):
    first = line_layer('LINESTRING(0 0, 10 0)', name='lakes')
    other = line_layer('LINESTRING(5 0, 15 0)', name='countries')
    for layer in (first, other):
        QgsProject.instance().addMapLayer(layer)
    add(dock, name, first, other)
    assert [item.text(0) for item in rule_items(dock)] == [label]


@both_engines
def test_the_list_keeps_with_for_points(dock: Dock):
    first = point_layer('POINT(0 0)', name='wells')
    other = point_layer('POINT(0 0)', name='pumps')
    for layer in (first, other):
        QgsProject.instance().addMapLayer(layer)
    add(dock, 'point/must-not-overlap-with', first, other)
    assert [item.text(0) for item in rule_items(dock)] == [
        'Must not overlap with pumps'
    ]


@both_engines
def test_the_results_keep_with_and_the_other_layer(dock: Dock):
    first = line_layer('LINESTRING(0 0, 10 0)', name='lakes')
    other = line_layer('LINESTRING(5 0, 15 0)', name='countries')
    for layer in (first, other):
        QgsProject.instance().addMapLayer(layer)
    add(dock, 'line/must-not-overlap-with', first, other)
    click_run(dock)
    texts = [node_text(node) for node in result_nodes(dock, 0)]
    assert texts == ['lakes: Must not overlap with countries (1)']
