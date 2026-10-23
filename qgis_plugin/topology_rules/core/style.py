from __future__ import annotations

import collections.abc as c
import dataclasses
import functools
import math
import typing as t
from enum import Enum

from qgis.core import (
    Qgis,
    QgsCategorizedSymbolRenderer,
    QgsColorRamp,
    QgsFeature,
    QgsFillSymbol,
    QgsLineSymbol,
    QgsMapLayer,
    QgsMarkerSymbol,
    QgsPresetSchemeColorRamp,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProcessingLayerPostProcessorInterface,
    QgsRendererCategory,
    QgsSettings,
    QgsSymbol,
    QgsSymbolLayerUtils,
    QgsVectorLayer,
)
from qgis.PyQt.QtGui import QColor
from qgis.PyQt.QtXml import QDomDocument

from ..enums import Setting
from ..utils import not_none, tr
from .binary import display_name, named_with, plain_name, short_name

RED = '#e31a1c'
# Distinct from each other and from the red of the invalid geometries
PALETTE = (
    '#1f78b4',
    '#ff7f00',
    '#33a02c',
    '#6a3d9a',
    '#b15928',
    '#00bfc4',
    '#e7298a',
    '#666666',
    '#a6a600',
    '#4d4dff',
)
INVALID = 'Invalid geometry'
# How far the colour of a group is at least from the red of the invalid
# geometries, in CIELAB units (Delta E 1976). The palette is 39 or more away
APART = 35.0
# Where a colour ramp is looked at for the parts of it away from the red
SAMPLES = 256
# Marks the error layers that Processing loaded, which are drawn again when
# the user changes the style
STYLED = 'topology_rules/styled'


class ColourBy(str, Enum):
    """What gives the errors their colours: the rule of the list, the rule
    on any layer, the layer that was checked, or nothing."""

    RULE_AND_LAYER = 'rule_and_layer'
    RULE = 'rule'
    LAYER = 'layer'
    ONE = 'one'


class Size(str, Enum):
    SMALL = 'small'
    NORMAL = 'normal'
    LARGE = 'large'


@dataclasses.dataclass(frozen=True)
class Measures:
    """How big the errors are drawn, in millimetres: a point, the white ring
    around it, and a line or the outline of a polygon."""

    point: float
    ring: float
    line: float


MEASURES = {
    Size.SMALL: Measures(2, 0.3, 0.5),
    Size.NORMAL: Measures(3, 0.4, 0.8),
    Size.LARGE: Measures(4.5, 0.6, 1.4),
}

E = t.TypeVar('E', ColourBy, Size)


def chosen(setting: Setting, default: E) -> E:
    """What the user chose, for every project, or the default."""
    value = QgsSettings().value(setting.value, default.value)
    try:
        return type(default)(str(value))
    except ValueError:
        return default


def choose(setting: Setting, value: ColourBy | Size) -> None:
    QgsSettings().setValue(setting.value, value.value)


def colour_by() -> ColourBy:
    return chosen(Setting.COLOUR_BY, ColourBy.RULE_AND_LAYER)


def size() -> Size:
    return chosen(Setting.SIZE, Size.NORMAL)


def default_ramp() -> QgsColorRamp:
    """The colours of the groups when the user chose no others."""
    return QgsPresetSchemeColorRamp([QColor(color) for color in PALETTE])


def ramp_text(ramp: QgsColorRamp) -> str:
    """A colour ramp as XML, for the settings, and to tell two apart."""
    document = QDomDocument()
    document.appendChild(QgsSymbolLayerUtils.saveColorRamp('', ramp, document))
    return document.toString(-1)


def same_ramp(first: QgsColorRamp, second: QgsColorRamp) -> bool:
    """Whether two ramps have the same colours. Their texts can differ in
    the last digits of the stops, as when a ramp was turned round twice."""
    if ramp_text(first) == ramp_text(second):
        return True
    if first.type() != second.type() or first.count() != second.count():
        return False
    for index in range(SAMPLES + 1):
        one, other = first.color(index / SAMPLES), second.color(index / SAMPLES)
        if any(
            abs(a - b) > 1
            for a, b in zip(one.getRgb(), other.getRgb(), strict=True)
        ):
            return False
    return True


@functools.lru_cache(maxsize=8)
def read_ramp(text: str) -> QgsColorRamp | None:
    """The colour ramp of the XML, read once: a random ramp has new colours
    each time it is read, and the error layers of a run share theirs."""
    document = QDomDocument()
    document.setContent(text)
    return QgsSymbolLayerUtils.loadColorRamp(document.documentElement())


def colour_ramp() -> QgsColorRamp:
    """The colour ramp that the user chose, or the default one."""
    text = str(QgsSettings().value(Setting.RAMP.value, '') or '')
    found = read_ramp(text) if text else None
    return not_none((found or default_ramp()).clone())


def choose_ramp(ramp: QgsColorRamp) -> None:
    """Keeps the colour ramp, and forgets it when it is the default one."""
    text = ramp_text(ramp)
    if text == ramp_text(default_ramp()):
        QgsSettings().remove(Setting.RAMP.value)
    else:
        QgsSettings().setValue(Setting.RAMP.value, text)


def reset_style() -> None:
    """Goes back to the style of the plugin."""
    settings = QgsSettings()
    for setting in (Setting.COLOUR_BY, Setting.SIZE, Setting.RAMP):
        settings.remove(setting.value)


def symbol(
    kind: t.Any, color: QColor | str, size: Size = Size.NORMAL
) -> QgsSymbol:
    """A symbol that stands out on any map, for the geometry kind."""
    measures = MEASURES[size]
    encoded = QgsSymbolLayerUtils.encodeColor(QColor(color))
    if kind == Qgis.GeometryType.Point:
        return not_none(
            QgsMarkerSymbol.createSimple(
                {
                    'name': 'circle',
                    'color': encoded,
                    'outline_color': '#ffffff',
                    'outline_width': str(measures.ring),
                    'size': str(measures.point),
                }
            )
        )
    if kind == Qgis.GeometryType.Line:
        return not_none(
            QgsLineSymbol.createSimple(
                {'color': encoded, 'width': str(measures.line)}
            )
        )
    fill = not_none(
        QgsFillSymbol.createSimple(
            {'outline_color': encoded, 'outline_width': str(measures.line)}
        )
    )
    fill_color = QColor(color)
    fill_color.setAlpha(64)
    not_none(fill.symbolLayer(0)).setFillColor(fill_color)
    return fill


# Tells the errors of one rule on one layer from those of the others: the
# rule, the layer that was checked, and the second layer if there is one.
# The errors of features that were left out have no rule and make one group.
KEY = (
    "CASE WHEN coalesce(\"rule\", '') = '' THEN '' ELSE "
    "\"rule\" || '|' || coalesce(\"origin_layer\", '') || '|' || "
    'CASE WHEN coalesce("destination_layer", \'\') = '
    "coalesce(\"origin_layer\", '') THEN '' "
    'ELSE coalesce("destination_layer", \'\') END END'
)
# The errors of a rule, on any layer
RULE_KEY = 'coalesce("rule", \'\')'
# The errors of the layer that was checked. An error on no feature, such as a
# gap, does not say which layer it is on, and those make a group of their own
LAYER_KEY = (
    "CASE WHEN coalesce(\"rule\", '') = '' THEN '' "
    "ELSE 'layer|' || coalesce(\"origin_layer\", '') END"
)
# All the errors of the rules
ONE = 'errors'
ONE_KEY = f"CASE WHEN coalesce(\"rule\", '') = '' THEN '' ELSE '{ONE}' END"


def quoted(text: str) -> str:
    return "'" + text.replace("'", "''") + "'"


def field_text(feature: QgsFeature, name: str) -> str:
    return str(feature[name] or '')


def error_key(rule: str, origin: str, destination: str) -> str:
    """The group of an error, as KEY gives it for the layers."""
    if not rule:
        return ''
    other = '' if destination == origin else destination
    return f'{rule}|{origin}|{other}'


@dataclasses.dataclass(frozen=True)
class Grouping:
    """How errors fall into groups: an expression for the layers, and the
    same for a feature, which a group is told by."""

    expression: str
    key_of: c.Callable[[QgsFeature], str]


def rule_grouping() -> Grouping:
    """The errors of a rule on a layer are a group."""

    def key_of(feature: QgsFeature) -> str:
        return error_key(
            *(
                str(feature[name] or '')
                for name in ('rule', 'origin_layer', 'destination_layer')
            )
        )

    return Grouping(KEY, key_of)


def entry_grouping(
    counts: c.Sequence[int], keys: c.Sequence[str] = ()
) -> Grouping:
    """The errors of each rule of a run are a group, or are in the group
    given for the rule. The tool numbers the violations in the order of the
    rules, so a rule has the numbers after those of the rules before it, as
    many as it has errors. This is exact also where an error has no feature
    to say which layer it is about, as a gap does not."""
    ranges: list[tuple[int, int, str]] = []
    start = 0
    for index, count in enumerate(counts):
        if count:
            key = keys[index] if index < len(keys) else f'e{index}'
            ranges.append((start, start + count, key))
        start += count
    whens = ' '.join(
        f'WHEN "violation_id" >= {low} AND "violation_id" < {high} '
        f'THEN {quoted(key)}'
        for low, high, key in ranges
    )
    expression = (
        f"CASE WHEN coalesce(\"rule\", '') = '' THEN '' {whens} ELSE '' END"
    )

    def key_of(feature: QgsFeature) -> str:
        if not str(feature['rule'] or ''):
            return ''
        number = feature['violation_id']
        for low, high, key in ranges:
            if isinstance(number, int) and low <= number < high:
                return key
        return ''

    return Grouping(expression, key_of)


def key_label(key: str) -> str:
    """What a group is called when nobody said, which is the name of its
    rule."""
    return display_name(key.split('|')[0])


def rule_and_layer_label(key: str) -> str:
    """What the errors of a rule on a layer are called, as KEY gives them:
    'roads: Must not have dangles', and the second layer of a rule that
    takes two. Errors on no layer, such as gaps, have the whole name."""
    rule, _, layers = key.partition('|')
    origin, _, other = layers.rpartition('|')
    if other:
        title = named_with(plain_name(rule), other)
    else:
        title = short_name(rule) if origin else display_name(rule)
    return f'{origin}: {title}' if origin else title


def layer_label(key: str) -> str:
    """The name of a layer of the errors, as LAYER_KEY gives it."""
    return key.partition('|')[2] or tr('No layer')


@dataclasses.dataclass(frozen=True)
class Legend:
    """The groups that errors are drawn in: how they are told apart, what
    they are called, and their order, which gives their colours. A group
    without a name is called after its key."""

    grouping: Grouping
    labels: c.Mapping[str, str] = dataclasses.field(default_factory=dict)
    order: tuple[str, ...] = ()
    name_of: c.Callable[[str], str] = key_label


def entry_legend(
    counts: c.Sequence[int], groups: c.Sequence[tuple[str, str]]
) -> Legend:
    """The errors of each rule of a run in the group given for the rule,
    with the name of the group. The groups are in the order of the rules."""
    keys = [key for key, _ in groups]
    return Legend(
        entry_grouping(counts, keys), dict(groups), tuple(dict.fromkeys(keys))
    )


def field_legend(colour: ColourBy) -> Legend:
    """The groups as the fields of the errors tell them, for errors that
    nothing else is known of, such as those that Processing loads."""
    if colour is ColourBy.RULE:
        return Legend(
            Grouping(RULE_KEY, lambda feature: field_text(feature, 'rule'))
        )
    if colour is ColourBy.LAYER:

        def layer_of(feature: QgsFeature) -> str:
            if not field_text(feature, 'rule'):
                return ''
            return f'layer|{field_text(feature, "origin_layer")}'

        return Legend(Grouping(LAYER_KEY, layer_of), name_of=layer_label)
    if colour is ColourBy.ONE:

        def one_of(feature: QgsFeature) -> str:
            return ONE if field_text(feature, 'rule') else ''

        return Legend(Grouping(ONE_KEY, one_of), {ONE: tr('Errors')})
    return Legend(rule_grouping(), name_of=rule_and_layer_label)


def lab(color: QColor) -> tuple[float, float, float]:
    """A colour in CIELAB, where distances are much as the eye sees them."""

    def linear(value: float) -> float:
        if value <= 0.04045:
            return value / 12.92
        return ((value + 0.055) / 1.055) ** 2.4

    def f(value: float) -> float:
        if value > 216 / 24389:
            return value ** (1 / 3)
        return (24389 / 27 * value + 16) / 116

    r, g, b = (linear(v) for v in (color.redF(), color.greenF(), color.blueF()))
    x = f((0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047)
    y = f(0.2126 * r + 0.7152 * g + 0.0722 * b)
    z = f((0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883)
    return 116 * y - 16, 500 * (x - y), 200 * (y - z)


RED_LAB = lab(QColor(RED))


def apart(color: QColor) -> bool:
    """Whether a colour is told apart from the red of invalid geometries."""
    return math.dist(lab(color), RED_LAB) >= APART


def moved(color: QColor) -> QColor:
    """A colour near the red, darker until it is told apart from it."""
    for factor in range(110, 1000, 10):
        darker = color.darker(factor)
        if apart(darker):
            return darker
    return QColor('#000000')


def group_colours(ramp: QgsColorRamp, count: int) -> list[QColor]:
    """The colours of a number of groups, none of them near the red of the
    invalid geometries. The colours of a list go round, past those near the
    red. A ramp is spread over the groups short of its ends, which are near
    white in many ramps, and only over its parts away from the red."""
    if isinstance(ramp, QgsPresetSchemeColorRamp) and ramp.colors():
        listed = ramp.colors()
        kept = [color for color in listed if apart(color)]
        kept = kept or [moved(color) for color in listed]
        return [kept[index % len(kept)] for index in range(count)]
    seen = [ramp.color(index / SAMPLES) for index in range(SAMPLES + 1)]
    away = [color for color in seen if apart(color)]
    found: list[QColor] = []
    for index in range(count):
        share = (index + 0.5) / count
        color = ramp.color(share)
        if len(away) == len(seen) and apart(color):
            found.append(color)
        elif away:
            found.append(away[min(int(share * len(away)), len(away) - 1)])
        else:
            found.append(moved(color))
    return found


def colors(
    keys: c.Iterable[str], ramp: QgsColorRamp | None = None
) -> dict[str, QColor]:
    """A colour for each group, from the colour ramp, and red for the
    invalid geometries, which have no rule."""
    ramp = colour_ramp() if ramp is None else ramp
    named = [key for key in dict.fromkeys(keys) if key]
    found = dict(zip(named, group_colours(ramp, len(named)), strict=True))
    found[''] = QColor(RED)
    return found


def present_keys(layer: QgsVectorLayer, grouping: Grouping) -> set[str]:
    """The groups that a layer has errors of, all of them also while the map
    shows only some."""
    if layer.fields().indexOf('rule') < 0:
        return {''}
    subset = layer.subsetString()
    if subset:
        layer.setSubsetString('')
    try:
        return {
            grouping.key_of(feature)
            for feature in t.cast('c.Iterable[QgsFeature]', layer.getFeatures())
        }
    finally:
        if subset:
            layer.setSubsetString(subset)


def style_layer(
    layer: QgsVectorLayer, legends: c.Mapping[ColourBy, Legend] | None = None
) -> None:
    """Draws the errors of a layer in the colour of their group, so that the
    groups can be told apart on the map, and at the size, that the user
    chose. The legend has the groups that the layer has errors of. A run
    gives its groups for each way of colouring, in an order that gives the
    colours; without them the fields of the errors tell the groups."""
    colour = colour_by()
    legend = (legends or {}).get(colour) or field_legend(colour)
    present = present_keys(layer, legend.grouping)
    keys = [key for key in legend.order if key in present]
    keys += sorted(present - set(keys))
    palette = colors([*legend.order, *keys])
    chosen_size = size()
    categories = [
        QgsRendererCategory(
            key,
            symbol(layer.geometryType(), palette[key], chosen_size),
            INVALID
            if not key
            else legend.labels.get(key) or legend.name_of(key),
        )
        for key in keys
    ]
    layer.setRenderer(
        QgsCategorizedSymbolRenderer(legend.grouping.expression, categories)
    )
    layer.triggerRepaint()


class ErrorStyle(QgsProcessingLayerPostProcessorInterface):
    """Styles the error layers that Processing loads when a run is done."""

    def __init__(self) -> None:
        super().__init__()

    def postProcessLayer(
        self,
        layer: QgsMapLayer | None,
        context: QgsProcessingContext,
        feedback: QgsProcessingFeedback | None,
    ) -> None:
        if isinstance(layer, QgsVectorLayer):
            style_layer(layer)
            layer.setCustomProperty(STYLED, True)
