from __future__ import annotations

import collections.abc as c
from pathlib import Path

from qgis.core import (
    QgsFeatureRequest,
    QgsLayerTreeGroup,
    QgsProject,
    QgsVectorLayer,
)

from ..core.commands import ERROR_LAYERS
from ..core.style import Grouping, style_layer
from ..utils import not_none

GROUP_NAME = 'Topology errors'
# Marks the group and the layers that the plugin added, so that a group of
# the user with the same name is left alone
PROPERTY = 'topology_rules/errors'


def remove_errors() -> None:
    """Removes the error layers and the group of the previous run."""
    project = not_none(QgsProject.instance())
    project.removeMapLayers(
        [
            layer.id()
            for layer in project.mapLayers().values()
            if layer.customProperty(PROPERTY)
        ]
    )
    for group in not_none(project.layerTreeRoot()).findGroups():
        parent = group.parent()
        if group.customProperty(PROPERTY) and isinstance(
            parent, QgsLayerTreeGroup
        ):
            parent.removeChildNode(group)


def add_errors(
    path: Path,
    grouping: Grouping,
    labels: c.Mapping[str, str],
    order: c.Iterable[str],
) -> list[QgsVectorLayer]:
    """Adds the error layers of a run that have errors, in a group on top."""
    project = not_none(QgsProject.instance())
    group = not_none(
        not_none(project.layerTreeRoot()).insertGroup(0, GROUP_NAME)
    )
    group.setCustomProperty(PROPERTY, True)
    layers: list[QgsVectorLayer] = []
    for name in ERROR_LAYERS:
        kind = name.removeprefix('errors_')
        found = QgsVectorLayer(f'{path}|layername={name}', name, 'ogr')
        if not found.isValid() or found.featureCount() == 0:
            continue
        # A copy in memory, which the map can filter and count, and which
        # does not point into a folder that Processing empties
        layer = not_none(found.materialize(QgsFeatureRequest()))
        layer.setName(f'{kind.capitalize()} errors')
        layer.setCustomProperty(PROPERTY, True)
        style_layer(layer, grouping, labels, order)
        project.addMapLayer(layer, False)
        group.addLayer(layer)
        layers.append(layer)
    if not layers:
        parent = group.parent()
        if isinstance(parent, QgsLayerTreeGroup):
            parent.removeChildNode(group)
    return layers
