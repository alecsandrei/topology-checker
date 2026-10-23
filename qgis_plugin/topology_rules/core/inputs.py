from __future__ import annotations

import collections.abc as c
import dataclasses
import math
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
    QgsPoint,
    QgsProcessingContext,
    QgsProcessingFeatureSource,
    QgsProcessingFeedback,
    QgsVectorFileWriter,
)

from ..exceptions import RunError
from ..utils import QgsCsException, named
from .layers import Dataset, Input, wkb_kind

# How many features are read by their ids at once
BATCH = 2000


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


def not_finite(geometry: QgsGeometry) -> bool:
    """Whether a coordinate is not a number or is infinite. The box shows
    most of them at once, and a point is empty to QGIS when one of its
    coordinates is not a number, so it is the box that tells; the vertices
    tell about the others."""
    box = geometry.boundingBox()
    corners = (
        box.xMinimum(),
        box.yMinimum(),
        box.xMaximum(),
        box.yMaximum(),
    )
    # An empty geometry has no coordinates at all
    if all(math.isnan(value) for value in corners):
        return False
    if not box.isFinite():
        return True
    kind = wkb_kind(geometry.wkbType())
    if kind == 'line':
        return not math.isfinite(geometry.length())
    if kind == 'point' and not geometry.isMultipart():
        return False
    return any(
        not math.isfinite(vertex.x() + vertex.y())
        for vertex in t.cast('c.Iterable[QgsPoint]', geometry.vertices())
    )


def invalid_reason(geometry: QgsGeometry) -> str | None:
    """Why the tool would reject a feature, or none if it takes it: a point
    or a line with a coordinate that is not finite, or a polygon that is not
    valid."""
    if geometry.isNull():
        return None
    if wkb_kind(geometry.wkbType()) != 'polygon':
        return 'coordinates must be finite' if not_finite(geometry) else None
    if geometry.isEmpty() or geometry.isGeosValid():
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


def chosen(
    item: Input, feedback: QgsProcessingFeedback
) -> c.Iterator[QgsFeature]:
    """The features that were asked for, without their attributes, in the
    order of their ids, which is the order of the errors.

    QGIS gives the features of a selection in no set order, those of an area
    in the order of its index, and the new features of an edit session first,
    so those are read by their ids, a batch at a time.
    """
    ids = wanted(item)
    if ids is None:
        batches: c.Iterable[list[int] | None] = [None]
    else:
        batches = (ids[i : i + BATCH] for i in range(0, len(ids), BATCH))
    for batch in batches:
        request = QgsFeatureRequest()
        request.setSubsetOfAttributes([])
        if batch is None:
            found = features(item.source, request)
        else:
            request.setFilterFids(batch)
            found = sorted(features(item.source, request), key=QgsFeature.id)
        for feature in found:
            if feedback.isCanceled():
                raise RunError('Canceled')
            yield feature


def wanted(item: Input) -> list[int] | None:
    """The ids of the features that were asked for, in order, or none where
    the source gives its features in that order: the features that cross
    the area, whole, and among them the ones that were chosen."""
    if item.ordered:
        return None
    if item.area is None and item.ids is not None:
        return sorted(item.ids)
    request = QgsFeatureRequest()
    if item.area is not None:
        request.setFilterRect(item.area)
    request.setNoAttributes()
    scoped = getattr(Qgis, 'FeatureRequestFlag', None)
    request.setFlags(
        scoped.NoGeometry
        if scoped is not None
        else named(QgsFeatureRequest, 'NoGeometry')
    )
    return sorted(
        feature.id()
        for feature in features(item.source, request)
        if item.ids is None or feature.id() in item.ids
    )


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
    if not crs.isValid():
        # A layer without a CRS is one in no CRS, which the tool reads as
        # such, and not one in an "undefined" CRS, which it cannot parse
        options.layerOptions = ['SRID=-1']
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
    ids: list[int] = []
    skipped: list[Skipped] = []
    batch: list[QgsFeature] = []
    try:
        for feature in chosen(item, feedback):
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
            batch.append(exported)
            ids.append(feature.id())
            if len(batch) >= 2000:
                res = writer.addFeatures(batch)
                ok = res[0] if isinstance(res, tuple) else bool(res)
                if not ok:
                    raise RunError(
                        f'Could not export {item.name}: {writer.errorMessage()}'
                    )
                batch.clear()
        if batch:
            res = writer.addFeatures(batch)
            ok = res[0] if isinstance(res, tuple) else bool(res)
            if not ok:
                raise RunError(
                    f'Could not export {item.name}: {writer.errorMessage()}'
                )
            batch.clear()
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
