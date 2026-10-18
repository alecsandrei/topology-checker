from __future__ import annotations

import collections.abc as c
import dataclasses
import typing as t

from qgis.core import (
    Qgis,
    QgsCategorizedSymbolRenderer,
    QgsFeature,
    QgsFillSymbol,
    QgsLineSymbol,
    QgsMapLayer,
    QgsMarkerSymbol,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProcessingLayerPostProcessorInterface,
    QgsRendererCategory,
    QgsSymbol,
    QgsVectorLayer,
)
from qgis.PyQt.QtGui import QColor

from ..utils import not_none
from .binary import display_name

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


def symbol(kind: t.Any, color: str) -> QgsSymbol:
    """A symbol that stands out on any map, for the geometry kind."""
    if kind == Qgis.GeometryType.Point:
        return not_none(
            QgsMarkerSymbol.createSimple(
                {
                    'name': 'circle',
                    'color': color,
                    'outline_color': '#ffffff',
                    'outline_width': '0.4',
                    'size': '3',
                }
            )
        )
    if kind == Qgis.GeometryType.Line:
        return not_none(
            QgsLineSymbol.createSimple({'color': color, 'width': '0.8'})
        )
    fill = not_none(
        QgsFillSymbol.createSimple(
            {'outline_color': color, 'outline_width': '0.8'}
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


def entry_grouping(counts: c.Sequence[int]) -> Grouping:
    """The errors of each rule of a run are a group. The tool numbers the
    violations in the order of the rules, so a rule has the numbers after
    those of the rules before it, as many as it has errors. This is exact
    also where an error has no feature to say which layer it is about, as
    a gap does not."""
    ranges: list[tuple[int, int, str]] = []
    start = 0
    for index, count in enumerate(counts):
        if count:
            ranges.append((start, start + count, f'e{index}'))
        start += count
    whens = ' '.join(
        f'WHEN "violation_id" >= {low} AND "violation_id" < {high} '
        f"THEN '{key}'"
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


def colors(keys: c.Iterable[str]) -> dict[str, str]:
    """A colour for each group, red for the invalid geometries, which have
    no rule."""
    named = [key for key in dict.fromkeys(keys) if key]
    found = {key: PALETTE[i % len(PALETTE)] for i, key in enumerate(named)}
    found[''] = RED
    return found


def style_layer(
    layer: QgsVectorLayer,
    grouping: Grouping | None = None,
    labels: c.Mapping[str, str] | None = None,
    order: c.Iterable[str] = (),
) -> None:
    """Draws the errors of a layer in the colour of their group, so that the
    groups can be told apart on the map. The legend has the groups that the
    layer has errors of. The order is that of the groups of the run, which
    gives the colours."""
    grouping = grouping or rule_grouping()
    present: set[str] = set()
    if layer.fields().indexOf('rule') < 0:
        present = {''}
    else:
        for feature in t.cast('c.Iterable[QgsFeature]', layer.getFeatures()):
            present.add(grouping.key_of(feature))
    keys = [key for key in order if key in present]
    keys += sorted(present - set(keys))
    palette = colors(list(order) + keys)
    categories = [
        QgsRendererCategory(
            key,
            symbol(layer.geometryType(), palette[key]),
            INVALID if not key else (labels or {}).get(key, key_label(key)),
        )
        for key in keys
    ]
    layer.setRenderer(
        QgsCategorizedSymbolRenderer(grouping.expression, categories)
    )
    layer.triggerRepaint()


def key_label(key: str) -> str:
    """What a group is called when nobody said, which is the name of its
    rule."""
    return display_name(key.split('|')[0])


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
