from __future__ import annotations

import dataclasses
import typing as t
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsProviderRegistry,
    QgsUnitTypes,
    QgsVectorLayer,
)

from ..core.binary import Rule, named_with
from ..core.layers import CONTAINERS, project_layer
from ..utils import not_none


def where_is(layer: QgsVectorLayer) -> str:
    """Where the data of a layer is, to tell layers of one name apart."""
    if layer.providerType() != 'ogr':
        return layer.publicSource()
    parts = t.cast(
        'dict[str, t.Any]',
        not_none(QgsProviderRegistry.instance()).decodeUri(
            'ogr', layer.source()
        ),
    )
    path = str(parts.get('path') or layer.source())
    name = parts.get('layerName')
    return (
        f'{path}\n{name}' if name and Path(path).suffix in CONTAINERS else path
    )


def unit_of(layer: QgsVectorLayer) -> str:
    """The unit of distances in the layer, such as 'm' or '°'."""
    crs = layer.crs()
    if not crs.isValid():
        return ''
    unit = crs.mapUnits()
    if unit == Qgis.DistanceUnit.Degrees:
        return '°'
    return QgsUnitTypes.toAbbreviatedString(unit)


@dataclasses.dataclass
class Entry:
    """A rule of the list, on layers of the project given by their ids."""

    rule: Rule
    layer: str
    layer_name: str
    unit: str
    other: str | None = None
    other_name: str = ''
    errors: int | None = None
    checked: bool = True

    @property
    def key(self) -> tuple[str, str, str | None]:
        return (self.rule.name, self.layer, self.other)

    @property
    def title(self) -> str:
        """The rule, and the other layer of the rules that take two."""
        if self.other is None:
            return self.rule.short_name
        return named_with(self.rule.short_name, self.other_name)

    @property
    def missing(self) -> bool:
        """Whether a layer of the rule has left the project."""
        return project_layer(self.layer) is None or (
            self.other is not None and project_layer(self.other) is None
        )
