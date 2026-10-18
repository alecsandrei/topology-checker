"""The form of the dock: the layer first, then the rules that fit it."""

from __future__ import annotations

import collections.abc as c
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsProject, QgsVectorLayer
from qgis.PyQt.QtCore import QPoint
from qgis.PyQt.QtWidgets import QApplication

from topology_rules.panel.dock import Dock

from .helpers import (
    fake_executable,
    line_layer,
    point_layer,
    polygon_layer,
    wait_for,
)

MULTIPART = 'geometry/must-not-be-multipart'
CONTAINED = 'geometry/must-not-be-contained-by'
DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
KINDS = ['point', 'line', 'polygon']


def project_layers() -> dict[str, QgsVectorLayer]:
    layers = {
        'point': point_layer('POINT(0 0)', name='points'),
        'line': line_layer('LINESTRING(0 0, 1 1)', name='lines'),
        'polygon': polygon_layer(
            'POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))', name='polygons'
        ),
    }
    for layer in layers.values():
        QgsProject.instance().addMapLayer(layer)
    return layers


@pytest.fixture
def layers(dock: Dock) -> dict[str, QgsVectorLayer]:
    found = project_layers()
    dock.refresh()
    return found


def listed(dock: Dock) -> list[t.Any]:
    return [
        dock.rule_box.itemData(index) for index in range(dock.rule_box.count())
    ]


def use(dock: Dock, layer: QgsVectorLayer) -> None:
    dock.input_box.setLayer(layer)


def test_the_layer_is_the_first_row_of_the_form(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    dock.resize(300, 700)
    dock.show()
    QApplication.processEvents()
    origin = QPoint(0, 0)
    layer_top = dock.input_box.mapTo(dock, origin).y()
    rule_top = dock.rule_box.mapTo(dock, origin).y()
    assert layer_top < rule_top


def test_the_rule_box_is_not_editable(dock: Dock):
    assert not dock.rule_box.isEditable()


@pytest.mark.parametrize('kind', KINDS)
def test_only_rules_that_fit_the_layer_are_listed(
    dock: Dock, layers: dict[str, QgsVectorLayer], kind: str
):
    use(dock, layers[kind])
    rules = listed(dock)
    assert rules
    assert all(rule.layer in (kind, 'any') for rule in rules)


@pytest.mark.parametrize('kind', KINDS)
def test_rules_for_any_geometry_are_always_listed(
    dock: Dock, layers: dict[str, QgsVectorLayer], kind: str
):
    use(dock, layers[kind])
    names = {rule.name for rule in listed(dock)}
    assert {MULTIPART, CONTAINED} <= names


def test_a_rule_of_another_kind_is_not_listed(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    use(dock, layers['point'])
    assert DANGLES not in {rule.name for rule in listed(dock)}
    use(dock, layers['line'])
    assert DANGLES in {rule.name for rule in listed(dock)}


@pytest.mark.parametrize('kind', KINDS)
def test_rules_of_one_layer_come_before_rules_of_two(
    dock: Dock, layers: dict[str, QgsVectorLayer], kind: str
):
    use(dock, layers[kind])
    needs_other = [rule.other is not None for rule in listed(dock)]
    assert needs_other == sorted(needs_other)
    assert not all(needs_other)


def test_changing_the_layer_changes_the_list(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    use(dock, layers['point'])
    points = [rule.name for rule in listed(dock)]
    use(dock, layers['polygon'])
    assert [rule.name for rule in listed(dock)] != points


def test_the_rule_is_kept_when_it_still_fits(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    use(dock, layers['point'])
    dock.rule_box.setCurrentIndex(
        [rule.name for rule in listed(dock)].index(MULTIPART)
    )
    use(dock, layers['polygon'])
    rule = dock.current_rule()
    assert rule is not None
    assert rule.name == MULTIPART


def test_another_rule_is_chosen_when_it_no_longer_fits(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    use(dock, layers['line'])
    dock.rule_box.setCurrentIndex(
        [rule.name for rule in listed(dock)].index(DANGLES)
    )
    use(dock, layers['point'])
    rule = dock.current_rule()
    assert rule is not None
    assert rule.layer in ('point', 'any')


def test_the_other_layer_is_hidden_for_a_rule_of_one_layer(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    use(dock, layers['line'])
    dock.rule_box.setCurrentIndex(
        [rule.name for rule in listed(dock)].index(DANGLES)
    )
    assert dock.other_box.isHidden()
    assert dock.other_label.isHidden()


def test_the_other_layer_is_shown_for_a_rule_of_two_layers(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    use(dock, layers['point'])
    dock.rule_box.setCurrentIndex(
        [rule.name for rule in listed(dock)].index(COVERED)
    )
    assert not dock.other_box.isHidden()
    assert not dock.other_label.isHidden()


def test_the_other_layer_has_the_kind_the_rule_needs(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    use(dock, layers['point'])
    dock.rule_box.setCurrentIndex(
        [rule.name for rule in listed(dock)].index(COVERED)
    )
    others = [
        dock.other_box.layer(index) for index in range(dock.other_box.count())
    ]
    assert [layer.name() for layer in others] == ['lines']


def test_a_rule_for_any_geometry_takes_layers_of_every_kind(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    use(dock, layers['point'])
    dock.rule_box.setCurrentIndex(
        [rule.name for rule in listed(dock)].index(CONTAINED)
    )
    names = {
        dock.other_box.layer(index).name()
        for index in range(dock.other_box.count())
    }
    assert names == {'points', 'lines', 'polygons'}


def test_tables_are_not_layers_to_check(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    table = QgsVectorLayer('None', 'table', 'memory')
    QgsProject.instance().addMapLayer(table)
    names = {
        dock.input_box.layer(index).name()
        for index in range(dock.input_box.count())
    }
    assert 'table' not in names


def test_without_layers_the_form_is_disabled(dock: Dock):
    dock.refresh()
    for name in ('input_box', 'rule_box', 'other_box', 'add_button'):
        assert not getattr(dock, name).isEnabled(), name


def test_without_layers_the_form_says_what_to_do(dock: Dock):
    dock.refresh()
    assert not dock.hint.isHidden()
    assert dock.hint.text()


def test_a_layer_enables_the_form_and_removes_the_hint(dock: Dock):
    dock.refresh()
    project_layers()
    for name in ('input_box', 'rule_box', 'add_button'):
        assert getattr(dock, name).isEnabled(), name
    assert dock.hint.isHidden()


def test_removing_the_last_layer_disables_the_form_again(dock: Dock):
    layers = project_layers()
    dock.refresh()
    for layer in layers.values():
        QgsProject.instance().removeMapLayer(layer.id())
    assert not dock.add_button.isEnabled()
    assert not dock.hint.isHidden()


def test_the_rules_are_the_ones_the_tool_lists(
    dock: Dock,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    layers: dict[str, QgsVectorLayer],
):
    configure(fake_executable(tmp_path))
    dock.refresh()
    use(dock, layers['point'])
    # The tool may still be asked for its rules
    wait_for(lambda: bool(listed(dock)))
    assert [rule.name for rule in listed(dock)] == [COVERED]
    use(dock, layers['line'])
    assert [rule.name for rule in listed(dock)] == [DANGLES]


def test_the_description_follows_the_rule(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    use(dock, layers['line'])
    for index in range(dock.rule_box.count()):
        dock.rule_box.setCurrentIndex(index)
        rule = dock.current_rule()
        assert rule is not None
        assert dock.description.text() == rule.description


@pytest.mark.parametrize('kind', KINDS)
def test_the_rules_of_the_layer_come_first_then_any_then_two_layers(
    dock: Dock, layers: dict[str, QgsVectorLayer], kind: str
):
    use(dock, layers[kind])

    def place(rule: t.Any) -> int:
        if rule.other is not None:
            return 2
        return 0 if rule.layer == kind else 1

    places = [place(rule) for rule in listed(dock)]
    assert places == sorted(places)
    assert set(places) == {0, 1, 2}


def test_polygons_come_before_the_geometries_that_are_not_multipart(
    dock: Dock, layers: dict[str, QgsVectorLayer]
):
    use(dock, layers['polygon'])
    names = [rule.display_name for rule in listed(dock)]
    assert names.index('Polygons must not overlap') < names.index(
        'Geometries must not be multipart'
    )


# The order of the form on the screen, at the width of a narrow dock


def top(widget: t.Any, dock: Dock) -> int:
    return int(widget.mapTo(dock, QPoint(0, 0)).y())


def bottom(widget: t.Any, dock: Dock) -> int:
    return top(widget, dock) + int(widget.height())


@pytest.fixture
def shown_form(dock: Dock, layers: dict[str, QgsVectorLayer]) -> Dock:
    use(dock, layers['line'])
    dock.resize(300, 900)
    dock.show()
    QApplication.processEvents()
    return dock


def last_input(dock: Dock) -> t.Any:
    """The widget of the last row of inputs: the other layer when the rule
    has one, else the rule."""
    return dock.rule_box if dock.other_box.isHidden() else dock.other_box


def test_the_description_is_below_the_last_input_row(shown_form: Dock):
    assert top(shown_form.description, shown_form) >= bottom(
        last_input(shown_form), shown_form
    )


def test_nothing_is_between_the_last_input_row_and_the_description(
    shown_form: Dock,
):
    gap = top(shown_form.description, shown_form) - bottom(
        last_input(shown_form), shown_form
    )
    assert gap < 20


def test_the_description_is_above_the_add_button(shown_form: Dock):
    assert bottom(shown_form.description, shown_form) <= top(
        shown_form.add_button, shown_form
    )


@pytest.fixture
def shown_two_layer_form(dock: Dock, layers: dict[str, QgsVectorLayer]) -> Dock:
    use(dock, layers['point'])
    dock.rule_box.setCurrentIndex(
        [rule.name for rule in listed(dock)].index(COVERED)
    )
    dock.resize(300, 900)
    dock.show()
    QApplication.processEvents()
    assert not dock.other_box.isHidden()
    return dock


def test_the_description_is_below_the_other_layer_of_a_rule_of_two(
    shown_two_layer_form: Dock,
):
    form = shown_two_layer_form
    assert top(form.description, form) >= bottom(form.other_box, form)


def test_the_description_is_above_the_add_button_for_a_rule_of_two(
    shown_two_layer_form: Dock,
):
    form = shown_two_layer_form
    assert bottom(form.description, form) <= top(form.add_button, form)


def test_the_layer_is_the_first_row_of_the_page(shown_form: Dock):
    page = shown_form.tabs.currentWidget()
    assert int(shown_form.input_box.mapTo(page, QPoint(0, 0)).y()) < 40


@pytest.mark.parametrize(
    'name',
    ['rule_box', 'description', 'add_button', 'tolerance', 'rules_tree'],
)
def test_the_layer_is_above_everything_else_of_the_form(
    shown_form: Dock, name: str
):
    widget = getattr(shown_form, name)
    assert top(shown_form.input_box, shown_form) < top(widget, shown_form)
