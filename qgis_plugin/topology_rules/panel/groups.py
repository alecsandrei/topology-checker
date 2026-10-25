from __future__ import annotations

import collections.abc as c
from pathlib import Path

from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsFeatureRequest,
    QgsLayerTreeGroup,
    QgsProject,
    QgsVectorLayer,
)

from ..core.commands import ERROR_LAYERS
from ..core.style import STYLED, ColourBy, Legend, style_layer
from ..utils import not_none, tr

# Marks the group and the layers that the plugin added, so that a group of
# the user with the same name is left alone
PROPERTY = 'topology_rules/errors'


def group_name() -> str:
    """The name of the group of the error layers in the layer tree."""
    return tr('Topology errors')


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
    legends: c.Mapping[ColourBy, Legend],
    crs: QgsCoordinateReferenceSystem,
) -> dict[str, QgsVectorLayer]:
    """Adds the error layers of a run that have errors, in a group on top,
    drawn by the groups of the run, by family. They are in the CRS of the
    run, which the executable may write to the file as another one. Each
    error keeps the id that it has in the file."""
    project = not_none(QgsProject.instance())
    group = not_none(
        not_none(project.layerTreeRoot()).insertGroup(0, group_name())
    )
    group.setCustomProperty(PROPERTY, True)
    layers: dict[str, QgsVectorLayer] = {}
    for name in ERROR_LAYERS:
        kind = name.removeprefix('errors_')
        found = QgsVectorLayer(f'{path}|layername={name}', name, 'ogr')
        if not found.isValid() or found.featureCount() == 0:
            continue
        # A copy in memory, which the map can filter and count, and which
        # does not point into a folder that Processing empties. The features
        # keep their ids: the file numbers them from 1 as they are written,
        # and the copy as it reads them, in that order
        layer = not_none(found.materialize(QgsFeatureRequest()))
        if crs.isValid():
            layer.setCrs(crs)
        layer.setName(f'{kind.capitalize()} errors')
        layer.setCustomProperty(PROPERTY, True)
        style_layer(layer, legends)
        project.addMapLayer(layer, False)
        group.addLayer(layer)
        layers[kind] = layer
    if not layers:
        parent = group.parent()
        if isinstance(parent, QgsLayerTreeGroup):
            parent.removeChildNode(group)
    return layers


def restyle_errors(legends: c.Mapping[ColourBy, Legend]) -> None:
    """Draws the error layers of the last run again, by its groups, and
    those that Processing loaded, in the style that the user chose now."""
    for layer in not_none(QgsProject.instance()).mapLayers().values():
        if not isinstance(layer, QgsVectorLayer):
            continue
        if layer.customProperty(PROPERTY):
            style_layer(layer, legends)
        elif layer.customProperty(STYLED):
            style_layer(layer)
