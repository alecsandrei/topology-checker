from __future__ import annotations

import collections.abc as c
import dataclasses
import re
import typing as t
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsCoordinateReferenceSystem,
    QgsCoordinateTransform,
    QgsFeature,
    QgsFeatureRequest,
    QgsFeatureSource,
    QgsFields,
    QgsGeometry,
    QgsProcessingContext,
    QgsProcessingFeatureSource,
    QgsProcessingFeedback,
    QgsVectorFileWriter,
)

from ..exceptions import RunError
from ..utils import QgsCsException, named
from .layers import Dataset, Input, wkb_kind


@dataclasses.dataclass(frozen=True)
class Skipped:
    """A feature that was left out of an export because the tool would
    reject it."""

    fid: int
    geometry: QgsGeometry
    reason: str


@dataclasses.dataclass(frozen=True)
class Prepared:
    """Data in a file that the tool can read.

    The ids are those of the source features, in the order of the file,
    where the file does not have the ids of the source itself.
    """

    dataset: Dataset
    ids: tuple[int, ...] | None = None
    skipped: tuple[Skipped, ...] = ()


def invalid_reason(geometry: QgsGeometry) -> str | None:
    """Why the tool would reject a polygon, or none if it takes it."""
    if geometry.isNull() or geometry.isEmpty():
        return None
    if wkb_kind(geometry.wkbType()) != 'polygon' or geometry.isGeosValid():
        return None
    errors = geometry.validateGeometry()
    return errors[0].what() if errors else 'invalid polygon'


def features(
    source: QgsFeatureSource, request: QgsFeatureRequest
) -> c.Iterable[QgsFeature]:
    """The features of a source, with the invalid ones, which this plugin
    reports itself."""
    if isinstance(source, QgsProcessingFeatureSource):
        scoped = getattr(Qgis, 'ProcessingFeatureSourceFlag', None)
        skip = (
            scoped.SkipGeometryValidityChecks
            if scoped is not None
            else named(
                QgsProcessingFeatureSource, 'FlagSkipGeometryValidityChecks'
            )
        )
        return t.cast(
            'c.Iterable[QgsFeature]', source.getFeatures(request, skip)
        )
    return t.cast('c.Iterable[QgsFeature]', source.getFeatures(request))


def aborts_on_invalid(context: QgsProcessingContext) -> bool:
    """Whether the user asked to stop at the first invalid geometry."""
    scoped = getattr(Qgis, 'InvalidGeometryCheck', None)
    if scoped is not None:
        return bool(context.invalidGeometryCheck() == scoped.AbortOnInvalid)
    return bool(
        context.invalidGeometryCheck()
        == named(QgsFeatureRequest, 'GeometryAbortOnInvalid')
    )


def moves(
    source: QgsCoordinateReferenceSystem, target: QgsCoordinateReferenceSystem
) -> bool:
    """Whether data has to be reprojected to be in the target CRS."""
    return source.isValid() and target.isValid() and source != target


def export_dataset(
    item: Input,
    directory: Path,
    target: QgsCoordinateReferenceSystem | None,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
    skip: t.Mapping[int, str] | None = None,
) -> Prepared:
    """Writes the geometries of the input to a GeoPackage in the directory,
    in the target CRS, and remembers the source id of each feature.

    Features that the tool would reject are left out: the polygons that are
    not valid, and the features of the skip, which has the reasons.
    """
    stem = re.sub(r'[^\w.-]+', '_', item.name) or 'layer'
    path = directory / f'{stem}.gpkg'
    index = 1
    while path.exists():
        index += 1
        path = directory / f'{stem}_{index}.gpkg'
    crs = item.source.sourceCrs()
    transform = None
    if target is not None and moves(crs, target):
        feedback.pushInfo(
            f'Reprojecting {item.name} from {crs.authid()} to {target.authid()}'
        )
        transform = QgsCoordinateTransform(
            crs, target, context.transformContext()
        )
        crs = target
    options = QgsVectorFileWriter.SaveVectorOptions()
    options.driverName = 'GPKG'
    options.fileEncoding = 'UTF-8'
    options.layerName = stem
    # The tool reads geometries, and an attribute such as `fid` can clash
    writer = QgsVectorFileWriter.create(
        str(path),
        QgsFields(),
        item.source.wkbType(),
        crs,
        context.transformContext(),
        options,
    )
    if writer is None or (
        writer.hasError() != QgsVectorFileWriter.WriterError.NoError
    ):
        message = '' if writer is None else writer.errorMessage()
        raise RunError(f'Could not export {item.name}: {message}')
    request = QgsFeatureRequest()
    request.setSubsetOfAttributes([])
    # The part of the layer that was asked for: the features that cross the
    # area, whole, and among them the ones that were chosen
    if item.area is not None:
        request.setFilterRect(item.area)
    elif item.ids is not None:
        request.setFilterFids(list(item.ids))
    ids: list[int] = []
    skipped: list[Skipped] = []
    try:
        for feature in features(item.source, request):
            if feedback.isCanceled():
                raise RunError('Canceled')
            if item.ids is not None and feature.id() not in item.ids:
                continue
            geometry = feature.geometry()
            if transform is not None and not geometry.isNull():
                try:
                    geometry.transform(transform)
                except QgsCsException as error:
                    raise RunError(
                        f'Could not reproject {item.name}: {error}'
                    ) from error
            reason = (skip or {}).get(feature.id()) or invalid_reason(geometry)
            if reason is not None:
                if aborts_on_invalid(context):
                    raise RunError(
                        f'{item.name}, feature {feature.id()}: {reason}'
                    )
                skipped.append(Skipped(feature.id(), geometry, reason))
                continue
            exported = QgsFeature()
            exported.setGeometry(geometry)
            if not writer.addFeature(exported):
                raise RunError(
                    f'Could not export {item.name}: {writer.errorMessage()}'
                )
            ids.append(feature.id())
    finally:
        # Closes the file also when the export fails
        del writer
    return Prepared(Dataset(str(path), stem), tuple(ids), tuple(skipped))


def prepare(
    item: Input,
    directory: Path,
    target: QgsCoordinateReferenceSystem | None,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
    skip: t.Mapping[int, str] | None = None,
    direct: bool = True,
) -> Prepared:
    """The file for the tool: the layer's own file where that serves, else an
    export of the features, which leaves out those the tool would reject."""
    if not item.valid:
        raise RunError(
            f'The layer {item.name} is not valid, its data cannot be read.'
        )
    if item.gone:
        raise RunError(item.gone)
    if (
        direct
        and not skip
        and item.direct is not None
        and not (target is not None and moves(item.crs, target))
    ):
        return Prepared(item.direct)
    return export_dataset(item, directory, target, context, feedback, skip)
