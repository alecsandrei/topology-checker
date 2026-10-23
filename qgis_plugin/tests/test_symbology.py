"""Error layers are drawn by rule, wherever they come from."""

from __future__ import annotations

import typing as t

import pytest
from qgis.core import (
    Qgis,
    QgsCategorizedSymbolRenderer,
    QgsProcessingFeedback,
    QgsProcessingUtils,
    QgsProject,
    QgsSimpleMarkerSymbolLayerBase,
    QgsVectorLayer,
)

from topology_rules.core.engine import Engine
from topology_rules.panel.dock import Dock

from .helpers import (
    add,
    click_run,
    line_layer,
    point_layer,
    polygon_layer,
    run_algorithm,
)

RED = '#e31a1c'
DANGLES = 'line/must-not-have-dangles'
COVERED = 'point/must-be-covered-by-lines'
LINE_OVERLAPS = 'line/must-not-overlap'
POLYGON_OVERLAPS = 'polygon/must-not-overlap'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
OVERLAPPING_LINES = ('LINESTRING(0 0, 10 0)', 'LINESTRING(5 0, 15 0)')
OVERLAPPING_POLYGONS = (
    'POLYGON((0 0, 10 0, 10 10, 0 10, 0 0))',
    'POLYGON((5 5, 15 5, 15 15, 5 15, 5 5))',
)
BOWTIE = 'POLYGON((20 0, 30 10, 30 0, 20 10, 20 0))'
KINDS = {
    'point': ('line_must_not_have_dangles', CHAIN, Qgis.GeometryType.Point),
    'line': (
        'line_must_not_overlap',
        OVERLAPPING_LINES,
        Qgis.GeometryType.Line,
    ),
    'polygon': (
        'polygon_must_not_overlap',
        OVERLAPPING_POLYGONS,
        Qgis.GeometryType.Polygon,
    ),
}


@pytest.fixture(autouse=True)
def ready(run_engine: Engine, executable: str, provider: object) -> None:
    """The real executable and the provider, as in a QGIS session, and
    each of the two engines in turn."""


def circle() -> t.Any:
    shapes = (
        getattr(Qgis, 'MarkerShape', None) or QgsSimpleMarkerSymbolLayerBase
    )
    return shapes.Circle


def categories(layer: QgsVectorLayer) -> list[t.Any]:
    renderer = layer.renderer()
    assert isinstance(renderer, QgsCategorizedSymbolRenderer)
    return list(renderer.categories())


def colour(layer: QgsVectorLayer, category: t.Any) -> str:
    """What identifies the category on the map: the colour of its outline or
    of its symbol."""
    symbol = category.symbol()
    if layer.geometryType() == Qgis.GeometryType.Polygon:
        return str(symbol.symbolLayer(0).strokeColor().name())
    return str(symbol.color().name())


def project_layer(
    wkts: tuple[str, ...], polygons: bool, name: str = 'roads'
) -> QgsVectorLayer:
    layer = (
        polygon_layer(*wkts, name=name)
        if polygons
        else line_layer(*wkts, name=name)
    )
    QgsProject.instance().addMapLayer(layer)
    return layer


def error_layers_in_project() -> dict[Qgis.GeometryType, QgsVectorLayer]:
    found: dict[Qgis.GeometryType, QgsVectorLayer] = {}
    for layer in QgsProject.instance().mapLayers().values():
        if (
            isinstance(layer, QgsVectorLayer)
            and layer.fields().indexOf('violation_id') >= 0
        ):
            found[layer.geometryType()] = layer
    return found


@pytest.fixture
def run_of_three_kinds(dock: Dock) -> Dock:
    """Point, line and polygon errors from one run of the dock."""
    add(dock, DANGLES, project_layer(CHAIN, polygons=False))
    add(dock, LINE_OVERLAPS, project_layer(OVERLAPPING_LINES, polygons=False))
    add(
        dock,
        POLYGON_OVERLAPS,
        project_layer(OVERLAPPING_POLYGONS, polygons=True),
    )
    click_run(dock)
    return dock


@pytest.fixture
def two_point_rules(dock: Dock) -> Dock:
    """Point errors of two rules, on the same layer of errors."""
    roads = project_layer(CHAIN, polygons=False)
    stops = point_layer('POINT(5 0)', 'POINT(5 5)', name='stops')
    QgsProject.instance().addMapLayer(stops)
    add(dock, DANGLES, roads)
    add(dock, COVERED, stops, roads)
    click_run(dock)
    return dock


@pytest.mark.parametrize('kind', KINDS)
def test_the_dock_draws_its_error_layers_by_rule(
    run_of_three_kinds: Dock, kind: str
):
    layer = error_layers_in_project()[KINDS[kind][2]]
    assert categories(layer)


def test_an_entry_has_a_category_named_like_its_node(two_point_rules: Dock):
    layer = error_layers_in_project()[Qgis.GeometryType.Point]
    labels = {category.label() for category in categories(layer)}
    assert {
        'roads: Must not have dangles',
        'stops: Must be covered by lines with roads',
    } <= labels


def test_a_rule_has_one_category(two_point_rules: Dock):
    layer = error_layers_in_project()[Qgis.GeometryType.Point]
    labels = [category.label() for category in categories(layer)]
    assert len(labels) == len(set(labels))


def test_the_categories_have_distinct_colours(two_point_rules: Dock):
    layer = error_layers_in_project()[Qgis.GeometryType.Point]
    colours = [colour(layer, category) for category in categories(layer)]
    assert len(colours) == len(set(colours))


def test_the_same_rule_on_two_layers_has_two_categories(dock: Dock):
    add(dock, DANGLES, project_layer(CHAIN, polygons=False, name='roads'))
    add(dock, DANGLES, project_layer(CHAIN, polygons=False, name='tracks'))
    click_run(dock)
    layer = error_layers_in_project()[Qgis.GeometryType.Point]
    labels = {category.label() for category in categories(layer)}
    assert {
        'roads: Must not have dangles',
        'tracks: Must not have dangles',
    } <= labels


def test_a_layer_of_errors_has_only_the_categories_in_it(
    run_of_three_kinds: Dock,
):
    layers = error_layers_in_project()
    point = {c.label() for c in categories(layers[Qgis.GeometryType.Point])}
    line = {c.label() for c in categories(layers[Qgis.GeometryType.Line])}
    assert point == {'roads: Must not have dangles'}
    assert line == {'roads: Must not overlap'}


def test_invalid_geometry_is_a_red_category(dock: Dock):
    layer = polygon_layer(*OVERLAPPING_POLYGONS, BOWTIE, name='parcels')
    QgsProject.instance().addMapLayer(layer)
    add(dock, POLYGON_OVERLAPS, layer)
    click_run(dock)
    errors = error_layers_in_project()[Qgis.GeometryType.Polygon]
    invalid = [
        category
        for category in categories(errors)
        if 'Invalid geometry' in category.label()
    ]
    assert len(invalid) == 1
    assert colour(errors, invalid[0]) == RED


def test_only_invalid_geometry_is_that_red(dock: Dock):
    layer = polygon_layer(*OVERLAPPING_POLYGONS, BOWTIE, name='parcels')
    QgsProject.instance().addMapLayer(layer)
    add(dock, POLYGON_OVERLAPS, layer)
    click_run(dock)
    errors = error_layers_in_project()[Qgis.GeometryType.Polygon]
    others = [
        colour(errors, category)
        for category in categories(errors)
        if 'Invalid geometry' not in category.label()
    ]
    assert RED not in others


def test_the_markers_of_the_points_are_circles(two_point_rules: Dock):
    layer = error_layers_in_project()[Qgis.GeometryType.Point]
    for category in categories(layer):
        assert category.symbol().symbolLayer(0).shape() == circle()


def test_the_lines_are_wide_enough_to_see(run_of_three_kinds: Dock):
    layer = error_layers_in_project()[Qgis.GeometryType.Line]
    for category in categories(layer):
        assert category.symbol().width() >= 0.6


def test_the_polygons_are_mostly_see_through(run_of_three_kinds: Dock):
    layer = error_layers_in_project()[Qgis.GeometryType.Polygon]
    for category in categories(layer):
        fill = category.symbol().symbolLayer(0).fillColor()
        assert fill.alpha() <= 100


@pytest.mark.parametrize('kind', KINDS)
def test_processing_draws_the_error_layers_by_rule(kind: str):
    name, wkts, geometry = KINDS[kind]
    layer = project_layer(wkts, polygons=kind == 'polygon')
    ran = run_algorithm(name, {'INPUT': layer.id()})
    assert ran.ok, ran.log
    loaded = ran.context.layersToLoadOnCompletion()
    assert len(loaded) == 1
    for key, details in loaded.items():
        result = QgsProcessingUtils.mapLayerFromString(key, ran.context)
        assert result.geometryType() == geometry
        post = details.postProcessor()
        assert post is not None
        post.postProcessLayer(result, ran.context, QgsProcessingFeedback())
        assert categories(result)


def test_processing_names_the_category_after_the_rule():
    layer = project_layer(CHAIN, polygons=False)
    ran = run_algorithm('line_must_not_have_dangles', {'INPUT': layer.id()})
    for key, details in ran.context.layersToLoadOnCompletion().items():
        result = QgsProcessingUtils.mapLayerFromString(key, ran.context)
        details.postProcessor().postProcessLayer(
            result, ran.context, QgsProcessingFeedback()
        )
        labels = {category.label() for category in categories(result)}
        assert any('must not have dangles' in label.lower() for label in labels)
