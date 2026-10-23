"""The user chooses how the errors are drawn, for every project, from the
Style menu: what gives them their colours, their size and the colour ramp."""

from __future__ import annotations

import collections.abc as c
import contextlib
import math
import typing as t

import pytest
from qgis.core import (
    Qgis,
    QgsCategorizedSymbolRenderer,
    QgsExpression,
    QgsExpressionContext,
    QgsExpressionContextUtils,
    QgsFeature,
    QgsGradientColorRamp,
    QgsMarkerSymbol,
    QgsPresetSchemeColorRamp,
    QgsProcessingFeedback,
    QgsProcessingUtils,
    QgsProject,
    QgsSettings,
    QgsSingleSymbolRenderer,
    QgsStyle,
    QgsVectorLayer,
)
from qgis.gui import QgsColorButton
from qgis.PyQt.QtCore import QRect, Qt, QTimer
from qgis.PyQt.QtGui import QColor
from qgis.PyQt.QtWidgets import (
    QAction,
    QApplication,
    QDialog,
    QMainWindow,
    QMenu,
    QWidget,
)

from topology_rules.core.style import (
    APART,
    PALETTE,
    ColourBy,
    Size,
    choose,
    colors,
    colour_by,
    colour_ramp,
    lab,
    ramp_text,
    reset_style,
    size,
    style_layer,
)
from topology_rules.enums import Setting
from topology_rules.panel.dock import Dock
from topology_rules.panel.style_menu import StyleMenu
from topology_rules.utils import not_none

from .helpers import (
    add,
    click_run,
    line_layer,
    point_layer,
    polygon_layer,
    select_result,
    start_algorithm,
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
COLOURS = {
    ColourBy.RULE_AND_LAYER: 'Rule and layer',
    ColourBy.RULE: 'Rule',
    ColourBy.LAYER: 'Layer',
    ColourBy.ONE: 'One colour',
}
SIZES = {Size.SMALL: 'Small', Size.NORMAL: 'Normal', Size.LARGE: 'Large'}


@pytest.fixture(autouse=True)
def default_style() -> c.Iterator[None]:
    """Starts and ends with the style of the plugin."""
    reset_style()
    yield
    reset_style()


@pytest.fixture
def menu() -> c.Iterator[StyleMenu]:
    menu = StyleMenu()
    yield menu
    menu.deleteLater()


class Spy:
    """Counts the signals it gets."""

    def __init__(self) -> None:
        self.calls = 0

    def __call__(self) -> None:
        self.calls += 1


def item(menu: QMenu, text: str) -> QAction:
    """The item of the menu with the text, or of a menu in it."""
    for action in menu.actions():
        if action.text() == text:
            return action
    for action in menu.actions():
        inner = action.menu()
        if inner is not None:
            with contextlib.suppress(LookupError):
                return item(inner, text)
    raise LookupError(text)


def pick(menu: StyleMenu, text: str) -> None:
    """Clicks an item of the Style menu, as a user does: the menu is opened
    first, then the submenu with the item."""
    menu.aboutToShow.emit()
    menu.ramp_menu.aboutToShow.emit()
    item(menu, text).trigger()


def checked(menu: QMenu) -> list[str]:
    return [action.text() for action in menu.actions() if action.isChecked()]


def checked_ramps(menu: StyleMenu) -> tuple[list[str], list[str]]:
    """The ramps that the ramp menu and its menu of all ramps check."""
    menu.ramp_menu.aboutToShow.emit()
    return checked(menu.ramp_menu), checked(menu.all_menu)


def style_ramp(name: str) -> t.Any:
    return not_none(not_none(QgsStyle.defaultStyle()).colorRamp(name))


def distance_from_red(color: QColor) -> float:
    return math.dist(lab(color), lab(QColor(RED)))


def answer_dialog(answer: c.Callable[[QDialog], None]) -> list[str]:
    """Answers the next dialog that opens, and gives its title. The
    dialog itself is gone once it is closed."""
    seen: list[str] = []

    def look() -> None:
        dialog = QApplication.activeModalWidget()
        if not isinstance(dialog, QDialog):
            QTimer.singleShot(10, look)
            return
        seen.append(dialog.windowTitle())
        answer(dialog)

    QTimer.singleShot(0, look)
    return seen


def categories(layer: QgsVectorLayer) -> list[t.Any]:
    renderer = layer.renderer()
    assert isinstance(renderer, QgsCategorizedSymbolRenderer)
    return list(renderer.categories())


def labels(layer: QgsVectorLayer) -> set[str]:
    return {category.label() for category in categories(layer)}


def colour(layer: QgsVectorLayer, category: t.Any) -> str:
    symbol = category.symbol()
    if layer.geometryType() == Qgis.GeometryType.Polygon:
        return str(symbol.symbolLayer(0).strokeColor().name())
    return str(symbol.color().name())


def colours(layer: QgsVectorLayer) -> dict[str, str]:
    """The colour of each category, by its name."""
    return {
        category.label(): colour(layer, category)
        for category in categories(layer)
    }


def drawn_in(layer: QgsVectorLayer) -> list[str]:
    """The category that each error is drawn in, as the map tells it."""
    renderer = layer.renderer()
    assert isinstance(renderer, QgsCategorizedSymbolRenderer)
    expression = QgsExpression(renderer.classAttribute())
    context = QgsExpressionContext(
        QgsExpressionContextUtils.globalProjectLayerScopes(layer)
    )
    values = {
        category.value(): category.label() for category in categories(layer)
    }
    found: list[str] = []
    for feature in t.cast('c.Iterable[t.Any]', layer.getFeatures()):
        context.setFeature(feature)
        found.append(values[expression.evaluate(context)])
    return found


def error_layers() -> dict[Qgis.GeometryType, QgsVectorLayer]:
    found: dict[Qgis.GeometryType, QgsVectorLayer] = {}
    for layer in not_none(QgsProject.instance()).mapLayers().values():
        if (
            isinstance(layer, QgsVectorLayer)
            and layer.fields().indexOf('violation_id') >= 0
        ):
            found[layer.geometryType()] = layer
    return found


def in_project(layer: QgsVectorLayer) -> QgsVectorLayer:
    not_none(QgsProject.instance()).addMapLayer(layer)
    return layer


@pytest.fixture
def three_groups(dock: Dock) -> QgsVectorLayer:
    """Point errors of one rule on two layers, and of another rule, in one
    run of the dock."""
    roads = in_project(line_layer(*CHAIN, name='roads'))
    tracks = in_project(line_layer(*CHAIN, name='tracks'))
    stops = in_project(point_layer('POINT(5 0)', 'POINT(5 5)', name='stops'))
    add(dock, DANGLES, roads)
    add(dock, DANGLES, tracks)
    add(dock, COVERED, stops, roads)
    click_run(dock)
    return error_layers()[Qgis.GeometryType.Point]


@pytest.fixture
def three_kinds(dock: Dock) -> dict[Qgis.GeometryType, QgsVectorLayer]:
    """Point, line and polygon errors from one run of the dock."""
    add(dock, DANGLES, in_project(line_layer(*CHAIN, name='roads')))
    add(
        dock,
        LINE_OVERLAPS,
        in_project(line_layer(*OVERLAPPING_LINES, name='rails')),
    )
    add(
        dock,
        POLYGON_OVERLAPS,
        in_project(polygon_layer(*OVERLAPPING_POLYGONS, name='parcels')),
    )
    click_run(dock)
    return error_layers()


@pytest.fixture
def with_invalid(dock: Dock) -> QgsVectorLayer:
    """Polygon errors of a rule, and a feature that was left out as
    invalid."""
    parcels = polygon_layer(*OVERLAPPING_POLYGONS, BOWTIE, name='parcels')
    add(dock, POLYGON_OVERLAPS, in_project(parcels))
    click_run(dock)
    return error_layers()[Qgis.GeometryType.Polygon]


def measures(layer: QgsVectorLayer) -> list[float]:
    """How big the errors of a layer are drawn: the size and the ring of a
    point, the width of a line, the outline of a polygon."""
    # The category owns the symbol
    category = categories(layer)[0]
    symbol = category.symbol()
    kind = layer.geometryType()
    if kind == Qgis.GeometryType.Point:
        return [symbol.size(), symbol.symbolLayer(0).strokeWidth()]
    if kind == Qgis.GeometryType.Line:
        return [symbol.width()]
    return [symbol.symbolLayer(0).strokeWidth()]


def processing_errors(name: str = 'roads') -> QgsVectorLayer:
    """The layer of dangles that Processing loads and has the plugin style,
    as it does when the task is done, while the algorithm is there."""
    roads = in_project(line_layer(*CHAIN, name=name))
    started = start_algorithm(
        'line_must_not_have_dangles', {'INPUT': roads.id()}
    )
    loaded: list[QgsVectorLayer] = []

    def load(ok: bool, results: object) -> None:
        context = started.context
        for key, details in context.layersToLoadOnCompletion().items():
            layer = QgsProcessingUtils.mapLayerFromString(key, context)
            not_none(context.temporaryLayerStore()).takeMapLayer(layer)
            assert isinstance(layer, QgsVectorLayer)
            loaded.append(in_project(layer))
            post = details.postProcessor()
            assert post is not None
            post.postProcessLayer(layer, context, QgsProcessingFeedback())

    started.task.executed.connect(load)
    ran = started.ran()
    assert ran.ok, ran.log
    [layer] = loaded
    return layer


def test_the_menu_has_the_style_items(menu: StyleMenu):
    assert menu.title() == 'Style'
    assert [action.text() for action in menu.actions()] == [
        'Colour by',
        'Size',
        'Colour ramp',
        '',
        'Reset style',
    ]
    assert [action.text() for action in menu.colour_menu.actions()] == list(
        COLOURS.values()
    )
    assert [action.text() for action in menu.size_menu.actions()] == list(
        SIZES.values()
    )


def test_the_menu_checks_the_style_of_the_plugin(menu: StyleMenu):
    menu.aboutToShow.emit()
    assert checked(menu.colour_menu) == ['Rule and layer']
    assert checked(menu.size_menu) == ['Normal']


def test_the_menu_checks_what_the_user_chose_before(menu: StyleMenu):
    choose(Setting.COLOUR_BY, ColourBy.LAYER)
    choose(Setting.SIZE, Size.LARGE)
    menu.aboutToShow.emit()
    assert checked(menu.colour_menu) == ['Layer']
    assert checked(menu.size_menu) == ['Large']


@pytest.mark.parametrize('chosen', list(COLOURS)[1:])
def test_a_colour_choice_is_kept_for_every_project(
    menu: StyleMenu, chosen: ColourBy
):
    spy = Spy()
    menu.changed.connect(spy)
    pick(menu, COLOURS[chosen])
    assert colour_by() is chosen
    assert QgsSettings().value(Setting.COLOUR_BY.value) == chosen.value
    assert checked(menu.colour_menu) == [COLOURS[chosen]]
    assert spy.calls == 1


@pytest.mark.parametrize('chosen', [Size.SMALL, Size.LARGE])
def test_a_size_is_kept_for_every_project(menu: StyleMenu, chosen: Size):
    spy = Spy()
    menu.changed.connect(spy)
    pick(menu, SIZES[chosen])
    assert size() is chosen
    assert QgsSettings().value(Setting.SIZE.value) == chosen.value
    assert checked(menu.size_menu) == [SIZES[chosen]]
    assert spy.calls == 1


@pytest.mark.parametrize('text', ['Rule and layer', 'Normal', 'Default'])
def test_choosing_what_is_chosen_changes_nothing(menu: StyleMenu, text: str):
    spy = Spy()
    menu.changed.connect(spy)
    pick(menu, text)
    assert spy.calls == 0
    settings = QgsSettings()
    for setting in (Setting.COLOUR_BY, Setting.SIZE, Setting.RAMP):
        assert settings.value(setting.value) is None


def test_choosing_what_is_chosen_keeps_a_style_of_the_user(
    dock: Dock, three_groups: QgsVectorLayer
):
    pick(dock.style_menu, 'Layer')
    pick(dock.style_menu, 'Viridis')
    # As the user styles the layer in its properties
    three_groups.setRenderer(QgsSingleSymbolRenderer(QgsMarkerSymbol()))
    for text in ('Layer', 'Normal', 'Viridis'):
        pick(dock.style_menu, text)
    assert isinstance(three_groups.renderer(), QgsSingleSymbolRenderer)


def test_one_item_of_a_submenu_is_checked(menu: StyleMenu):
    pick(menu, 'Small')
    pick(menu, 'Large')
    pick(menu, 'Rule')
    pick(menu, 'One colour')
    assert checked(menu.size_menu) == ['Large']
    assert checked(menu.colour_menu) == ['One colour']


def test_a_setting_that_is_not_known_is_the_default():
    settings = QgsSettings()
    settings.setValue(Setting.COLOUR_BY.value, 'by mood')
    settings.setValue(Setting.SIZE.value, 'huge')
    settings.setValue(Setting.RAMP.value, '<not a ramp/>')
    assert colour_by() is ColourBy.RULE_AND_LAYER
    assert size() is Size.NORMAL
    assert isinstance(colour_ramp(), QgsPresetSchemeColorRamp)


def test_the_default_ramp_has_the_colours_of_the_plugin():
    ramp = colour_ramp()
    assert isinstance(ramp, QgsPresetSchemeColorRamp)
    assert [color.name() for color in ramp.colors()] == list(PALETTE)


def test_the_ramp_menu_has_the_default_and_the_favourite_ramps(
    menu: StyleMenu,
):
    style = not_none(QgsStyle.defaultStyle())
    favourites = style.symbolsOfFavorite(QgsStyle.StyleEntity.ColorrampEntity)
    assert 'Viridis' in favourites
    menu.ramp_menu.aboutToShow.emit()
    texts = [action.text() for action in menu.ramp_menu.actions()]
    assert texts == [
        'Default',
        *favourites,
        'All ramps',
        '',
        'Invert',
        'Edit…',
    ]
    assert not menu.ramp_menu.icon().isNull()
    for action in menu.ramp_menu.actions()[: len(favourites) + 1]:
        assert action.isCheckable()
        assert not action.icon().isNull()


def test_all_ramps_has_every_ramp_of_the_style(menu: StyleMenu):
    menu.ramp_menu.aboutToShow.emit()
    names = not_none(QgsStyle.defaultStyle()).colorRampNames()
    actions = menu.all_menu.actions()
    assert [action.text() for action in actions] == names
    assert all(not action.icon().isNull() for action in actions)


def test_the_ramp_in_use_is_checked(menu: StyleMenu):
    assert checked_ramps(menu) == (['Default'], [])
    pick(menu, 'Viridis')
    assert checked_ramps(menu) == (['Viridis'], ['Viridis'])
    # Not a favourite
    pick(menu, 'Plasma')
    assert checked_ramps(menu) == ([], ['Plasma'])
    pick(menu, 'Default')
    assert checked_ramps(menu) == (['Default'], [])


def test_invert_turns_the_ramp_round(menu: StyleMenu):
    pick(menu, 'Viridis')
    spy = Spy()
    menu.changed.connect(spy)
    pick(menu, 'Invert')
    viridis = style_ramp('Viridis')
    ramp = colour_ramp()
    assert ramp.color(0).name() == viridis.color(1).name()
    assert ramp.color(1).name() == viridis.color(0).name()
    assert checked_ramps(menu) == ([], [])
    pick(menu, 'Invert')
    assert checked_ramps(menu) == (['Viridis'], ['Viridis'])
    assert spy.calls == 2


def test_edit_opens_the_ramp_editor_and_keeps_the_ramp(menu: StyleMenu):
    pick(menu, 'Viridis')
    spy = Spy()
    menu.changed.connect(spy)

    def edit(dialog: QDialog) -> None:
        # The two colours of the gradient, as the user sets them
        for name, color in (('btnColor1', '#000000'), ('btnColor2', '#ffffff')):
            button = dialog.findChild(QgsColorButton, name)
            assert isinstance(button, QgsColorButton)
            button.setColor(QColor(color))
        dialog.accept()

    seen = answer_dialog(edit)
    pick(menu, 'Edit…')
    assert seen == ['Colour ramp']
    assert spy.calls == 1
    assert colour_ramp().color(0).name() == '#000000'
    assert colour_ramp().color(1).name() == '#ffffff'


@pytest.mark.parametrize('answer', [QDialog.accept, QDialog.reject])
def test_an_editor_left_alone_changes_nothing(
    menu: StyleMenu, answer: c.Callable[[QDialog], None]
):
    pick(menu, 'Viridis')
    before = ramp_text(colour_ramp())
    spy = Spy()
    menu.changed.connect(spy)
    seen = answer_dialog(answer)
    pick(menu, 'Edit…')
    assert len(seen) == 1
    assert spy.calls == 0
    assert ramp_text(colour_ramp()) == before


def placed_editor(menu: StyleMenu) -> tuple[QWidget, QRect]:
    """The window that the editor of the ramp opens over, and where it
    opens, the first time: then it opens where it was left."""
    settings = QgsSettings()
    for key in settings.allKeys():
        if key.startswith('Windows/') and key.endswith('/geometry'):
            settings.remove(key)
    seen: list[tuple[QWidget, QRect]] = []

    def look(dialog: QDialog) -> None:
        parent = not_none(dialog.parentWidget())
        seen.append((not_none(parent.window()), dialog.frameGeometry()))
        dialog.reject()

    answer_dialog(look)
    pick(menu, 'Edit…')
    assert len(seen) == 1
    return seen[0]


def assert_centred(inner: QRect, outer: QRect) -> None:
    centre, middle = inner.center(), outer.center()
    assert abs(centre.x() - middle.x()) <= 2, (inner, outer)
    assert abs(centre.y() - middle.y()) <= 2, (inner, outer)


# Within the screen of the tests, of 800 by 600, as the dialog is kept in it
@pytest.mark.parametrize('left', [20, 140])
def test_the_ramp_editor_opens_over_the_main_window(
    dock: Dock, window: QMainWindow, left: int
):
    window.addDockWidget(Qt.DockWidgetArea.RightDockWidgetArea, dock)
    window.setGeometry(left, 20, 640, 480)
    window.show()
    owner, placed = placed_editor(dock.style_menu)
    assert owner is window
    assert_centred(placed, window.frameGeometry())


def test_the_ramp_editor_opens_over_a_floating_panel(
    dock: Dock, window: QMainWindow
):
    window.addDockWidget(Qt.DockWidgetArea.RightDockWidgetArea, dock)
    window.show()
    dock.setFloating(True)
    dock.setGeometry(150, 100, 500, 450)
    owner, placed = placed_editor(dock.style_menu)
    assert owner is dock
    assert_centred(placed, dock.frameGeometry())


def test_a_submenu_of_style_opens_the_editor_over_the_panel(dock: Dock):
    outer = QMenu(dock)
    inner = StyleMenu(outer)
    outer.addMenu(inner)
    assert inner.window_of_editor() is dock.window()


def test_the_ramp_menu_only_chooses_ramps(menu: StyleMenu):
    """It does not make or save ramps of the style."""
    menu.ramp_menu.aboutToShow.emit()
    names = set(not_none(QgsStyle.defaultStyle()).colorRampNames())
    texts = {action.text() for action in menu.ramp_menu.actions()}
    texts |= {action.text() for action in menu.all_menu.actions()}
    assert texts - names == {'Default', 'All ramps', '', 'Invert', 'Edit…'}


def test_a_ramp_of_the_style_is_kept_for_every_project(menu: StyleMenu):
    spy = Spy()
    menu.changed.connect(spy)
    pick(menu, 'Viridis')
    assert spy.calls == 1
    assert QgsSettings().value(Setting.RAMP.value)
    viridis = not_none(not_none(QgsStyle.defaultStyle()).colorRamp('Viridis'))
    # Read again, as by the next session
    kept = colour_ramp()
    assert [kept.color(at / 4).name() for at in range(5)] == [
        viridis.color(at / 4).name() for at in range(5)
    ]


def test_the_default_ramp_item_brings_the_palette_back(menu: StyleMenu):
    pick(menu, 'Viridis')
    pick(menu, 'Default')
    assert QgsSettings().value(Setting.RAMP.value) is None
    assert isinstance(colour_ramp(), QgsPresetSchemeColorRamp)


def test_reset_goes_back_to_the_style_of_the_plugin(menu: StyleMenu):
    pick(menu, 'Layer')
    pick(menu, 'Large')
    pick(menu, 'Viridis')
    spy = Spy()
    menu.changed.connect(spy)
    pick(menu, 'Reset style')
    assert spy.calls == 1
    settings = QgsSettings()
    for setting in (Setting.COLOUR_BY, Setting.SIZE, Setting.RAMP):
        assert settings.value(setting.value) is None
    assert checked(menu.colour_menu) == ['Rule and layer']
    assert checked(menu.size_menu) == ['Normal']
    assert checked_ramps(menu) == (['Default'], [])


@pytest.mark.parametrize(
    ('chosen', 'expected'),
    [
        (
            ColourBy.RULE_AND_LAYER,
            {
                'roads: Must not have dangles',
                'tracks: Must not have dangles',
                'stops: Must be covered by lines with roads',
            },
        ),
        (
            ColourBy.RULE,
            {'Lines must not have dangles', 'Points must be covered by lines'},
        ),
        (ColourBy.LAYER, {'roads', 'tracks', 'stops'}),
        (ColourBy.ONE, {'Errors'}),
    ],
)
def test_the_run_is_drawn_as_the_user_chose(
    dock: Dock,
    three_groups: QgsVectorLayer,
    chosen: ColourBy,
    expected: set[str],
):
    pick(dock.style_menu, COLOURS[chosen])
    assert labels(three_groups) == expected
    assert len(set(colours(three_groups).values())) == len(expected)


def test_every_error_is_in_a_group_of_the_legend(
    dock: Dock, three_groups: QgsVectorLayer
):
    errors = three_groups.featureCount()
    pick(dock.style_menu, 'Rule')
    assert sorted(drawn_in(three_groups)) == sorted(
        ['Lines must not have dangles'] * (errors - 1)
        + ['Points must be covered by lines']
    )
    pick(dock.style_menu, 'Layer')
    assert drawn_in(three_groups).count('stops') == 1


def test_a_run_is_drawn_as_the_user_chose_before(dock: Dock):
    choose(Setting.COLOUR_BY, ColourBy.LAYER)
    add(dock, DANGLES, in_project(line_layer(*CHAIN, name='roads')))
    click_run(dock)
    assert labels(error_layers()[Qgis.GeometryType.Point]) == {'roads'}


@pytest.mark.parametrize('chosen', list(COLOURS))
def test_invalid_geometries_stay_red(
    dock: Dock, with_invalid: QgsVectorLayer, chosen: ColourBy
):
    pick(dock.style_menu, COLOURS[chosen])
    found = colours(with_invalid)
    assert found.pop('Invalid geometry') == RED
    assert found
    assert RED not in found.values()


def test_the_colours_are_those_of_cielab():
    for color, expected in (
        ('#ff0000', (53.24, 80.09, 67.2)),
        ('#ffffff', (100, 0, 0)),
        ('#000000', (0, 0, 0)),
        ('#0000ff', (32.3, 79.19, -107.86)),
    ):
        assert lab(QColor(color)) == pytest.approx(expected, abs=0.05)


def test_no_ramp_of_the_style_gives_a_group_the_red_of_invalid_geometries():
    style = not_none(QgsStyle.defaultStyle())
    names = style.colorRampNames()
    assert 'Reds' in names
    for name in names:
        ramp = style_ramp(name)
        for count in range(1, 13):
            keys = [f'group {index}' for index in range(count)]
            found = colors(keys, ramp)
            assert found.pop('').name() == RED
            for color in found.values():
                assert distance_from_red(color) >= APART, (name, count)


def test_two_groups_of_the_reds_are_not_red():
    found = colors(['roads', 'tracks'], style_ramp('Reds'))
    assert found['tracks'].name() != '#d32020'
    assert len({color.name() for color in found.values()}) == 3


@pytest.mark.parametrize(
    'ramp',
    [
        QgsGradientColorRamp(QColor(RED), QColor('#d01010')),
        QgsPresetSchemeColorRamp([QColor(RED), QColor('#ef3b2c')]),
    ],
)
def test_a_ramp_of_reds_only_is_made_darker(ramp: t.Any):
    found = colors(['roads', 'tracks', 'rails'], ramp)
    found.pop('')
    for color in found.values():
        assert distance_from_red(color) >= APART


def test_a_run_in_the_reds_tells_invalid_geometries_apart(
    dock: Dock, with_invalid: QgsVectorLayer
):
    pick(dock.style_menu, 'Reds')
    found = colours(with_invalid)
    assert found.pop('Invalid geometry') == RED
    assert found
    for name in found.values():
        assert distance_from_red(QColor(name)) >= APART


def test_the_sizes_go_from_small_to_large(
    dock: Dock, three_kinds: dict[Qgis.GeometryType, QgsVectorLayer]
):
    found: dict[Qgis.GeometryType, list[list[float]]] = {}
    for text in SIZES.values():
        pick(dock.style_menu, text)
        for kind, layer in three_kinds.items():
            found.setdefault(kind, []).append(measures(layer))
    for small, normal, large in found.values():
        assert all(
            a < b < c for a, b, c in zip(small, normal, large, strict=True)
        )


def test_the_normal_size_is_that_of_before(
    three_kinds: dict[Qgis.GeometryType, QgsVectorLayer],
):
    assert measures(three_kinds[Qgis.GeometryType.Point]) == [3, 0.4]
    assert measures(three_kinds[Qgis.GeometryType.Line]) == [0.8]
    assert measures(three_kinds[Qgis.GeometryType.Polygon]) == [0.8]


def test_the_groups_take_the_colours_of_the_ramp(
    dock: Dock, three_groups: QgsVectorLayer
):
    pick(dock.style_menu, 'Viridis')
    viridis = not_none(not_none(QgsStyle.defaultStyle()).colorRamp('Viridis'))
    spread = [viridis.color((index + 0.5) / 3).name() for index in range(3)]
    assert sorted(colours(three_groups).values()) == sorted(spread)


def test_the_groups_take_the_colours_of_the_plugin(
    dock: Dock, three_groups: QgsVectorLayer
):
    assert sorted(colours(three_groups).values()) == sorted(PALETTE[:3])


def test_the_colours_of_the_plugin_go_round():
    keys = [f'group {index}' for index in range(len(PALETTE) + 2)]
    found = colors(keys)
    assert found[keys[len(PALETTE)]].name() == PALETTE[0]
    assert found[keys[len(PALETTE) + 1]].name() == PALETTE[1]
    assert found[''].name() == RED


def test_reset_draws_over_a_style_of_the_user(
    dock: Dock, three_groups: QgsVectorLayer
):
    pick(dock.style_menu, 'Layer')
    pick(dock.style_menu, 'Large')
    # As the user styles the layer in its properties
    three_groups.setRenderer(QgsSingleSymbolRenderer(QgsMarkerSymbol()))
    pick(dock.style_menu, 'Reset style')
    assert labels(three_groups) == {
        'roads: Must not have dangles',
        'tracks: Must not have dangles',
        'stops: Must be covered by lines with roads',
    }
    assert measures(three_groups) == [3, 0.4]


def test_a_filtered_map_keeps_every_group_in_the_legend(
    dock: Dock, three_groups: QgsVectorLayer
):
    select_result(dock, 'tracks: Must not have dangles')
    subset = three_groups.subsetString()
    assert subset
    pick(dock.style_menu, 'Layer')
    assert labels(three_groups) == {'roads', 'tracks', 'stops'}
    assert three_groups.subsetString() == subset


def test_the_style_leaves_other_layers_alone(
    dock: Dock, three_groups: QgsVectorLayer
):
    roads = next(
        layer
        for layer in not_none(QgsProject.instance()).mapLayers().values()
        if layer.name() == 'roads'
    )
    before = roads.renderer().type()
    pick(dock.style_menu, 'One colour')
    assert roads.renderer().type() == before


@pytest.mark.parametrize(
    ('chosen', 'expected'),
    [
        (ColourBy.RULE_AND_LAYER, {'roads: Must not have dangles'}),
        (ColourBy.RULE, {'Lines must not have dangles'}),
        (ColourBy.LAYER, {'roads'}),
        (ColourBy.ONE, {'Errors'}),
    ],
)
def test_processing_draws_as_the_user_chose(
    executable: str, provider: object, chosen: ColourBy, expected: set[str]
):
    choose(Setting.COLOUR_BY, chosen)
    choose(Setting.SIZE, Size.LARGE)
    errors = processing_errors()
    assert labels(errors) == expected
    assert measures(errors) == [4.5, 0.6]


def test_processing_takes_the_colours_of_the_ramp(
    executable: str, provider: object, menu: StyleMenu
):
    pick(menu, 'Viridis')
    errors = processing_errors()
    viridis = not_none(not_none(QgsStyle.defaultStyle()).colorRamp('Viridis'))
    assert list(colours(errors).values()) == [viridis.color(0.5).name()]


def test_layers_that_processing_loaded_follow_the_style(
    dock: Dock, executable: str, provider: object
):
    errors = processing_errors()
    pick(dock.style_menu, 'Layer')
    assert labels(errors) == {'roads'}
    pick(dock.style_menu, 'Large')
    assert measures(errors) == [4.5, 0.6]
    pick(dock.style_menu, 'Reset style')
    assert labels(errors) == {'roads: Must not have dangles'}
    assert measures(errors) == [3, 0.4]


def test_without_a_run_the_layers_tell_the_groups(
    dock: Dock, three_groups: QgsVectorLayer
):
    """As after the project was opened again, with its error layers."""
    dock.legends = {}
    pick(dock.style_menu, 'Rule')
    pick(dock.style_menu, 'Rule and layer')
    # A stop that no line covers has no feature of the roads to name them
    assert labels(three_groups) == {
        'roads: Must not have dangles',
        'tracks: Must not have dangles',
        'stops: Must be covered by lines',
    }
    assert len(set(colours(three_groups).values())) == 3
    pick(dock.style_menu, 'Layer')
    assert labels(three_groups) == {'roads', 'tracks', 'stops'}


def test_a_new_project_forgets_the_groups_of_the_run(
    dock: Dock, three_groups: QgsVectorLayer
):
    assert dock.legends
    not_none(QgsProject.instance()).clear()
    assert dock.legends == {}


def errors_without_a_layer() -> QgsVectorLayer:
    """Errors of gaps, which are on no feature, and one on a feature."""
    layer = QgsVectorLayer(
        'Polygon?crs=EPSG:32633&field=rule:string&field=origin_layer:string'
        '&field=destination_layer:string&field=violation_id:integer',
        'errors',
        'memory',
    )
    features = []
    for number, origin in enumerate((None, 'parcels')):
        feature = QgsFeature(layer.fields())
        feature.setAttributes(
            ['polygon/must-not-have-gaps', origin, None, number]
        )
        features.append(feature)
    not_none(layer.dataProvider()).addFeatures(features)
    return layer


@pytest.mark.parametrize(
    ('chosen', 'expected'),
    [
        (
            ColourBy.RULE_AND_LAYER,
            {'parcels: Must not have gaps', 'Polygons must not have gaps'},
        ),
        (ColourBy.LAYER, {'parcels', 'No layer'}),
        (ColourBy.ONE, {'Errors'}),
    ],
)
def test_errors_on_no_layer_have_a_group_of_their_own(
    chosen: ColourBy, expected: set[str]
):
    layer = errors_without_a_layer()
    choose(Setting.COLOUR_BY, chosen)
    style_layer(layer)
    assert labels(layer) == expected
