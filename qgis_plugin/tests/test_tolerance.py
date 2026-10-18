"""The tolerance of the dock: its default, its unit and its use."""

from __future__ import annotations

import pytest
from qgis.core import QgsProject, QgsVectorLayer
from qgis.gui import QgsDoubleSpinBox

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    add_features,
    click_run,
    select_rules,
    set_checked,
    shown,
)

DANGLES = 'line/must-not-have-dangles'


def lines_in(crs: str, name: str) -> QgsVectorLayer:
    layer = QgsVectorLayer(f'LineString?crs={crs}', name, 'memory')
    add_features(layer, ['LINESTRING(0 0, 10 0)', 'LINESTRING(10.4 0, 20 0)'])
    QgsProject.instance().addMapLayer(layer)
    return layer


def test_the_tolerance_is_a_spin_box_with_a_default(dock: Dock):
    assert isinstance(dock.tolerance, QgsDoubleSpinBox)
    assert dock.tolerance.specialValueText().startswith('Default')
    assert dock.tolerance.value() == dock.tolerance.minimum()


def test_clearing_the_tolerance_goes_back_to_the_default(dock: Dock):
    dock.tolerance.setValue(2.5)
    dock.tolerance.clear()
    assert dock.tolerance.value() == dock.tolerance.minimum()


def test_a_tolerance_changes_the_run(dock: Dock):
    layer = lines_in('EPSG:32633', 'gap')
    add(dock, DANGLES, layer)
    click_run(dock)
    assert shown(dock) == 4
    dock.tolerance.setValue(1.0)
    click_run(dock)
    assert shown(dock) == 2


def test_the_default_tolerance_is_used_after_clearing(dock: Dock):
    add(dock, DANGLES, lines_in('EPSG:32633', 'gap'))
    dock.tolerance.setValue(1.0)
    click_run(dock)
    dock.tolerance.clear()
    click_run(dock)
    assert shown(dock) == 4


@pytest.mark.parametrize(
    ('crs', 'suffix'), [('EPSG:3857', ' m'), ('EPSG:4326', ' °')]
)
def test_the_suffix_is_the_unit_of_the_first_layer(
    dock: Dock, crs: str, suffix: str
):
    add(dock, DANGLES, lines_in(crs, 'first'))
    assert dock.tolerance.suffix() == suffix


def test_the_suffix_follows_the_first_rule_of_the_list(dock: Dock):
    metres = lines_in('EPSG:3857', 'metres')
    degrees = lines_in('EPSG:4326', 'degrees')
    add(dock, DANGLES, metres)
    add(dock, DANGLES, degrees)
    assert dock.tolerance.suffix() == ' m'
    select_rules(dock, 0)
    dock.remove_rules()
    assert dock.tolerance.suffix() == ' °'


def test_the_suffix_changes_when_the_list_is_replaced(dock: Dock):
    add(dock, DANGLES, lines_in('EPSG:3857', 'metres'))
    dock.clear_rules()
    add(dock, DANGLES, lines_in('EPSG:4326', 'degrees'))
    assert dock.tolerance.suffix() == ' °'


@pytest.mark.parametrize(
    ('crs', 'text'),
    [
        ('EPSG:32633', 'Default (1 mm)'),
        ('EPSG:3857', 'Default (1 mm)'),
        ('EPSG:4326', 'Default (1e-8°)'),
    ],
)
def test_the_default_says_what_it_is_for_the_first_layer(
    dock: Dock, crs: str, text: str
):
    add(dock, DANGLES, lines_in(crs, 'first'))
    assert dock.tolerance.specialValueText() == text


def test_the_default_follows_the_first_layer_of_the_list(dock: Dock):
    add(dock, DANGLES, lines_in('EPSG:3857', 'metres'))
    add(dock, DANGLES, lines_in('EPSG:4326', 'degrees'))
    assert dock.tolerance.specialValueText() == 'Default (1 mm)'
    select_rules(dock, 0)
    dock.remove_rules()
    assert dock.tolerance.specialValueText() == 'Default (1e-8°)'


def unchecked_degrees(dock: Dock) -> None:
    """Rules on a layer in degrees, which is the first of the list but is not
    checked, and on a layer in metres, which is the first that runs."""
    add(dock, DANGLES, lines_in('EPSG:4326', 'degrees'))
    add(dock, DANGLES, lines_in('EPSG:3857', 'metres'))
    assert [entry.layer_name for entry in dock.entries] == ['degrees', 'metres']
    set_checked(dock, 0, False)
    assert [entry.checked for entry in dock.entries] == [False, True]


def test_the_suffix_is_the_unit_of_the_first_checked_rule(dock: Dock):
    unchecked_degrees(dock)
    assert dock.tolerance.suffix() == ' m'


def test_the_default_follows_the_first_checked_rule(dock: Dock):
    unchecked_degrees(dock)
    assert dock.tolerance.specialValueText() == 'Default (1 mm)'


def test_the_suffix_goes_back_when_the_first_rule_is_checked_again(dock: Dock):
    unchecked_degrees(dock)
    set_checked(dock, 0, True)
    assert dock.tolerance.suffix() == ' °'
    assert dock.tolerance.specialValueText() == 'Default (1e-8°)'
