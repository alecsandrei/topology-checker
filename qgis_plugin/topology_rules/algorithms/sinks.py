from __future__ import annotations

import collections.abc as c
import typing as t

from qgis.core import (
    Qgis,
    QgsCoordinateReferenceSystem,
    QgsFeature,
    QgsFeatureSink,
    QgsProcessingContext,
    QgsProcessingParameterFeatureSink,
    QgsVectorLayer,
)

from .._typing import Parameters
from ..core.checks import ID_COLUMNS, IdMap, source_id
from ..core.invalid import Invalid, error_fields
from ..core.layers import geometry_word
from ..core.style import ErrorStyle
from ..utils import QgsProcessingException, source_type, tr
from .base import Planned

# What Processing calls a layer that is not saved
TEMPORARY_OUTPUT = 'TEMPORARY_OUTPUT'

# The sink of each geometry the errors can have: its name, the layer of the
# tool and the geometry that is written, with the type for Processing
SINKS = (
    ('ERRORS_POINT', 'point', 'MultiPoint', 'VectorPoint'),
    ('ERRORS_LINE', 'line', 'MultiLineString', 'VectorLine'),
    ('ERRORS_POLYGON', 'polygon', 'MultiPolygon', 'VectorPolygon'),
)


def sink_label(kind: str) -> str:
    """The name of the sink of the errors of the geometry, in Processing."""
    return {
        'point': tr('Point errors'),
        'line': tr('Line errors'),
        'polygon': tr('Polygon errors'),
    }[kind]


def loaded_name(title: str, kind: str) -> str:
    """The name of the layer of the errors of the geometry that a run of
    the tool loads, such as 'Lines must not have dangles (line)'."""
    return tr('{title} ({geometry})').format(
        title=title, geometry=geometry_word(kind)
    )


class SinkAlgorithm(Planned):
    """An algorithm whose errors come out as a sink for each geometry."""

    def add_sinks(self) -> None:
        for name, kind, _, sink_type in SINKS:
            self.addParameter(
                QgsProcessingParameterFeatureSink(
                    name,
                    sink_label(kind),
                    source_type(sink_type),
                    TEMPORARY_OUTPUT,
                )
            )

    def sink_files(
        self, parameters: Parameters, context: QgsProcessingContext
    ) -> list[str]:
        """Where the sinks are going to be written."""
        return [
            self.parameterAsOutputLayer(parameters, name, context)
            for name, *_ in SINKS
        ]

    def write_sinks(
        self,
        parameters: Parameters,
        context: QgsProcessingContext,
        crs: QgsCoordinateReferenceSystem,
        path: str,
        ids: IdMap,
        invalid: c.Sequence[Invalid],
        title: str,
    ) -> Parameters:
        """Copies the errors of the tool to the sinks, and loads the sinks
        that have errors."""
        fields = error_fields()
        names = fields.names()
        results: dict[str, t.Any] = {}
        destinations: list[tuple[str, int]] = []
        for name, kind, geometry, _ in SINKS:
            sink, destination = self.parameterAsSink(
                parameters,
                name,
                context,
                fields,
                getattr(Qgis.WkbType, geometry),
                crs,
            )
            if sink is None:
                raise QgsProcessingException(
                    self.invalidSinkError(parameters, name)
                )
            written = 0
            errors = QgsVectorLayer(
                f'{path}|layername=errors_{kind}', kind, 'ogr'
            )
            if errors.isValid():
                present = set(errors.fields().names())
                for error in t.cast(
                    'c.Iterable[QgsFeature]', errors.getFeatures()
                ):
                    row = [error[n] if n in present else None for n in names]
                    for layer_column, id_column in ID_COLUMNS:
                        index = names.index(id_column)
                        row[index] = source_id(
                            ids, row[names.index(layer_column)], row[index]
                        )
                    copy = QgsFeature(fields)
                    copy.setGeometry(error.geometry())
                    copy.setAttributes(row)
                    if not sink.addFeature(
                        copy, QgsFeatureSink.Flag.FastInsert
                    ):
                        raise QgsProcessingException(
                            tr('Could not write the {kind} errors').format(
                                kind=geometry_word(kind)
                            )
                        )
                    written += 1
            for item in invalid:
                if item.kind == kind:
                    sink.addFeature(item.feature(fields))
                    written += 1
            del sink
            results[name] = destination
            destinations.append((str(destination), written))
        self.load_sinks(context, destinations, title)
        return results

    def load_sinks(
        self,
        context: QgsProcessingContext,
        destinations: list[tuple[str, int]],
        title: str,
    ) -> None:
        """Loads the sinks that have errors and draws them in red, and
        nothing else."""
        loads = t.cast(
            'dict[str | None, QgsProcessingContext.LayerDetails]',
            dict(context.layersToLoadOnCompletion()),
        )
        for destination, written in destinations:
            if not written:
                loads.pop(destination, None)
        context.setLayersToLoadOnCompletion(loads)
        # Processing owns the styles, but Python must not drop them
        self.styles: list[ErrorStyle] = []
        for (name, kind, *_), (destination, written) in zip(
            SINKS, destinations, strict=True
        ):
            if not written:
                continue
            context.addLayerToLoadOnCompletion(
                destination,
                QgsProcessingContext.LayerDetails(
                    loaded_name(title, kind), context.project(), name
                ),
            )
            # The details of the context own the style, so the style is
            # set on them and not on a copy
            style = ErrorStyle()
            self.styles.append(style)
            context.layerToLoadOnCompletionDetails(
                destination
            ).setPostProcessor(style)
