"""Features with an invalid geometry: reported, not fatal."""

from __future__ import annotations

import collections.abc as c
import json
import typing as t
from pathlib import Path

import pytest
from qgis.core import Qgis, QgsFeatureRequest, QgsProject, QgsVectorLayer

from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    click_run,
    errors_layer,
    layer_nodes,
    message_text,
    node_text,
    polygon_layer,
    queue_column,
    result_nodes,
    result_rows,
    run_algorithm,
    save,
    script_executable,
    select_result,
    shown,
)

OVERLAPS = 'polygon/must-not-overlap'
OVERLAPS_WITH = 'polygon/must-not-overlap-with'
SQUARE = 'POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))'
# Overlaps the square, so the layer has a real error
OVERLAPPING = 'POLYGON((2 2, 6 2, 6 6, 2 6, 2 2))'
# A ring that crosses itself
BOWTIE = 'POLYGON((20 0, 30 10, 30 0, 20 10, 20 0))'
ELSEWHERE = tuple(
    f'POLYGON(({x} 50, {x + 4} 50, {x + 4} 54, {x} 54, {x} 50))'
    for x in range(0, 60, 10)
)
INVALID_CHECK = getattr(Qgis, 'InvalidGeometryCheck', QgsFeatureRequest)
ABORT = (
    getattr(INVALID_CHECK, 'AbortOnInvalid', None)
    or QgsFeatureRequest.GeometryAbortOnInvalid
)


@pytest.fixture(autouse=True)
def ready(executable: str, provider: object) -> None:
    """The real executable and the provider, as in a QGIS session."""


def project_layer(layer: QgsVectorLayer) -> QgsVectorLayer:
    QgsProject.instance().addMapLayer(layer)
    return layer


def with_a_bowtie(name: str = 'parcels') -> QgsVectorLayer:
    layer = polygon_layer(SQUARE, OVERLAPPING, BOWTIE, name=name)
    return project_layer(layer)


def with_a_hole_in_the_ids() -> QgsVectorLayer:
    """Seven features, the bow-tie the last: the ids 2 and 4 are gone."""
    layer = polygon_layer(
        SQUARE,
        ELSEWHERE[0],
        OVERLAPPING,
        ELSEWHERE[1],
        ELSEWHERE[2],
        ELSEWHERE[3],
        BOWTIE,
        name='parcels',
    )
    assert layer.dataProvider().deleteFeatures([2, 4])
    return project_layer(layer)


def geopackage(tmp_path: Path) -> QgsVectorLayer:
    path = tmp_path / 'parcels.gpkg'
    save(with_a_bowtie(), path, 'GPKG', 'parcels')
    return project_layer(
        QgsVectorLayer(f'{path}|layername=parcels', 'parcels', 'ogr')
    )


def shapefile(tmp_path: Path) -> QgsVectorLayer:
    path = tmp_path / 'parcels.shp'
    save(with_a_bowtie(), path, 'ESRI Shapefile', 'parcels')
    return project_layer(QgsVectorLayer(str(path), 'parcels', 'ogr'))


ORIGINS: dict[str, c.Callable[[Path], QgsVectorLayer]] = {
    'memory': lambda tmp_path: with_a_bowtie(),
    'geopackage': geopackage,
    'shapefile': shapefile,
    'ids with a hole': lambda tmp_path: with_a_hole_in_the_ids(),
}


def bowtie_id(layer: QgsVectorLayer) -> int:
    """The id the layer itself gives to the invalid feature."""
    found = [
        feature.id()
        for feature in layer.getFeatures()
        if not feature.geometry().isGeosValid()
    ]
    assert len(found) == 1
    return found[0]


def invalid_rows(layer: QgsVectorLayer) -> list[t.Any]:
    return [
        feature
        for feature in layer.getFeatures()
        if feature['error_type'] == 'invalid'
    ]


def total(ran: t.Any) -> int:
    return sum(
        errors_layer(ran, name).featureCount()
        for name in ('ERRORS_POINT', 'ERRORS_LINE', 'ERRORS_POLYGON')
    )


def test_an_invalid_feature_does_not_fail_the_run():
    layer = with_a_bowtie()
    ran = run_algorithm('polygon_must_not_overlap', {'INPUT': layer.id()})
    assert ran.ok, ran.log


def test_the_real_error_is_still_found():
    layer = with_a_bowtie()
    ran = run_algorithm('polygon_must_not_overlap', {'INPUT': layer.id()})
    errors = errors_layer(ran, 'ERRORS_POLYGON')
    others = [f for f in errors.getFeatures() if f['error_type'] != 'invalid']
    assert len(others) == 1


@pytest.mark.parametrize('origin', ORIGINS)
def test_an_invalid_feature_is_one_row_of_the_polygon_errors(
    origin: str, tmp_path: Path
):
    layer = ORIGINS[origin](tmp_path)
    ran = run_algorithm('polygon_must_not_overlap', {'INPUT': layer.id()})
    assert ran.ok, ran.log
    rows = invalid_rows(errors_layer(ran, 'ERRORS_POLYGON'))
    assert len(rows) == 1
    row = rows[0]
    assert row['rule'] == ''
    assert row['origin_layer'] == layer.name()
    assert row['origin_fid'] == bowtie_id(layer)


@pytest.mark.parametrize('origin', ORIGINS)
def test_the_invalid_row_has_the_geometry_of_the_feature(
    origin: str, tmp_path: Path
):
    layer = ORIGINS[origin](tmp_path)
    ran = run_algorithm('polygon_must_not_overlap', {'INPUT': layer.id()})
    row = invalid_rows(errors_layer(ran, 'ERRORS_POLYGON'))[0]
    expected = layer.getFeature(bowtie_id(layer)).geometry()
    # The error layers hold multi-part geometries, as the tool writes them
    expected.convertToMultiType()
    assert row.geometry().asWkt(6) == expected.asWkt(6)


def test_the_count_includes_the_invalid_feature():
    layer = with_a_bowtie()
    ran = run_algorithm('polygon_must_not_overlap', {'INPUT': layer.id()})
    assert ran.results['COUNT'] == total(ran) == 2


def test_an_invalid_feature_of_the_other_layer_is_reported_against_it():
    first = project_layer(polygon_layer(SQUARE, name='first'))
    second = project_layer(polygon_layer(OVERLAPPING, BOWTIE, name='second'))
    ran = run_algorithm(
        'polygon_must_not_overlap_with',
        {'INPUT': first.id(), 'OTHER': second.id()},
    )
    assert ran.ok, ran.log
    rows = invalid_rows(errors_layer(ran, 'ERRORS_POLYGON'))
    assert len(rows) == 1
    assert rows[0]['origin_layer'] == 'second'
    assert rows[0]['origin_fid'] == bowtie_id(second)


def test_an_invalid_feature_of_the_input_of_a_two_layer_rule():
    first = project_layer(polygon_layer(SQUARE, BOWTIE, name='first'))
    second = project_layer(polygon_layer(OVERLAPPING, name='second'))
    ran = run_algorithm(
        'polygon_must_not_overlap_with',
        {'INPUT': first.id(), 'OTHER': second.id()},
    )
    assert ran.ok, ran.log
    rows = invalid_rows(errors_layer(ran, 'ERRORS_POLYGON'))
    assert [row['origin_layer'] for row in rows] == ['first']


def test_aborting_on_invalid_features_names_the_layer_and_the_feature():
    layer = with_a_hole_in_the_ids()
    ran = run_algorithm(
        'polygon_must_not_overlap',
        {'INPUT': layer.id()},
        invalid_check=ABORT,
    )
    assert not ran.ok
    assert 'parcels' in ran.log
    assert f'feature {bowtie_id(layer)}' in ran.log


def test_aborting_does_not_show_an_alias_or_an_export_id():
    layer = with_a_hole_in_the_ids()
    ran = run_algorithm(
        'polygon_must_not_overlap',
        {'INPUT': layer.id()},
        invalid_check=ABORT,
    )
    assert 'FID' not in ran.log
    assert 'Some(' not in ran.log


def test_a_queue_with_an_invalid_feature_runs(dock: Dock):
    add(dock, OVERLAPS, with_a_bowtie())
    click_run(dock)
    # The count of the rule leaves the invalid feature out; the results list it
    assert dock.entries[0].errors == 1
    assert shown(dock) == 2


def test_the_results_name_the_invalid_geometry(dock: Dock):
    add(dock, OVERLAPS, with_a_bowtie())
    click_run(dock)
    rules = [row[0] for row in result_rows(dock)]
    assert sorted(rules) == ['Invalid geometry', 'Must not overlap']


def test_the_results_have_a_node_for_the_invalid_geometries(dock: Dock):
    add(dock, OVERLAPS, with_a_bowtie())
    click_run(dock)
    nodes = [node_text(node) for node in result_nodes(dock, 0)]
    assert any(node.endswith('Invalid geometry (1)') for node in nodes)


def test_selecting_the_invalid_geometries_shows_only_them_on_the_map(
    dock: Dock,
):
    add(dock, OVERLAPS, with_a_bowtie())
    click_run(dock)
    select_result(dock, 'Invalid geometry')
    errors = next(
        layer
        for layer in QgsProject.instance().mapLayers().values()
        if layer.fields().indexOf('violation_id') >= 0
    )
    assert errors.featureCount() == 1


def test_the_message_bar_says_how_many_features_were_skipped(
    dock: Dock, bar: t.Any
):
    add(dock, OVERLAPS, with_a_bowtie())
    click_run(dock)
    text = message_text(bar)
    assert 'skipped' in text
    assert '1' in text


FAKE_RULES = [
    {
        'name': OVERLAPS,
        'description': 'Polygons must not overlap.',
        'arcgis': None,
        'qgis': None,
        'layer': 'polygon',
        'other': None,
    }
]


def test_a_feature_the_tool_rejects_is_named_by_layer_and_source_id(
    dock: Dock,
    tmp_path: Path,
    configure: c.Callable[[str | Path], None],
    bar: t.Any,
):
    # The tool knows the alias of the export and the number in the export:
    # the fourth feature of the file is the feature 5 of the layer, whose
    # feature 2 is gone. The message has to say the layer and its feature
    reason = 'self-intersection or closing spike'
    error = (
        f'layer my_parcels, FID 4, part 0, ring Some(0): '
        f'invalid polygon: {reason}'
    )
    reply = json.dumps(json.dumps({'error': error}))
    path = script_executable(
        tmp_path,
        f'print({reply}, file=sys.stderr)\nsys.exit(2)\n',
        rules=FAKE_RULES,
    )
    configure(path)
    dock.refresh()
    layer = project_layer(
        polygon_layer(SQUARE, *ELSEWHERE[:4], name='my parcels')
    )
    assert layer.dataProvider().deleteFeatures([2])
    add(dock, OVERLAPS, layer)
    click_run(dock)
    text = message_text(bar)
    assert f'my parcels, feature 5: invalid polygon: {reason}' in text
    for hidden in ('my_parcels', 'FID', 'Some(', str(path), '--json'):
        assert hidden not in text, hidden


# The list says what is invalid in a layer, apart from what a rule found


SECOND_BOWTIE = 'POLYGON((40 0, 50 10, 50 0, 40 10, 40 0))'


def with_two_bowties() -> QgsVectorLayer:
    layer = polygon_layer(
        SQUARE, OVERLAPPING, BOWTIE, SECOND_BOWTIE, name='parcels'
    )
    return project_layer(layer)


def test_the_count_of_a_rule_leaves_the_invalid_features_out(dock: Dock):
    add(dock, OVERLAPS, with_two_bowties())
    click_run(dock)
    assert queue_column(dock, 3) == ['1']


def test_the_layer_says_how_many_of_its_features_are_invalid(dock: Dock):
    add(dock, OVERLAPS, with_two_bowties())
    click_run(dock)
    (node,) = layer_nodes(dock)
    assert node.text(0) == 'parcels · 2 invalid'


def test_the_second_column_of_the_layer_row_is_not_text(dock: Dock):
    add(dock, OVERLAPS, with_two_bowties())
    click_run(dock)
    (node,) = layer_nodes(dock)
    assert node.text(1) == ''


def test_a_layer_with_invalid_features_shows_the_warning(dock: Dock):
    add(dock, OVERLAPS, with_two_bowties())
    (node,) = layer_nodes(dock)
    before = node.icon(0).cacheKey()
    click_run(dock)
    (node,) = layer_nodes(dock)
    assert node.icon(0).cacheKey() != before
    assert node.toolTip(0)


def test_a_layer_without_invalid_features_says_nothing(dock: Dock):
    layer = polygon_layer(SQUARE, OVERLAPPING, name='parcels')
    add(dock, OVERLAPS, project_layer(layer))
    click_run(dock)
    (node,) = layer_nodes(dock)
    assert node.text(0) == 'parcels'
    assert node.text(1) == ''
