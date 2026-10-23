from __future__ import annotations

import collections.abc as c
import dataclasses
import math
import re
import typing as t
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsAbstractFeatureSource,
    QgsAbstractGeometry,
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
    QgsWkbTypes,
)

from ..exceptions import RunError
from ..utils import QgsCsException, named, not_none
from .layers import Dataset, Input, wkb_kind

# How many features are read by their ids, or written, at once
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


def parts(geometry: QgsGeometry) -> c.Iterable[QgsAbstractGeometry]:
    return t.cast('c.Iterable[QgsAbstractGeometry]', geometry.constParts())


def finite(part: QgsAbstractGeometry) -> bool:
    """Whether the coordinates of a part are finite. A point whose
    coordinates are both not a number is an empty one, which has none."""
    if isinstance(part, QgsPoint):
        x, y = part.x(), part.y()
        if math.isnan(x) and math.isnan(y):
            return True
        return math.isfinite(x) and math.isfinite(y)
    # QGIS crashes going over the vertices of an empty polygon
    if part.isEmpty():
        return True
    return all(
        math.isfinite(vertex.x()) and math.isfinite(vertex.y())
        for vertex in t.cast('c.Iterable[QgsPoint]', part.vertices())
    )


def not_finite(geometry: QgsGeometry) -> bool:
    """Whether a coordinate is not a number or is infinite.

    The box holds every infinite coordinate, but it can miss one that is not
    a number. A point has its box for coordinates, and a straight line with
    such a one has a length that is not a number, as finite coordinates give
    a length that is finite or infinite. The vertices tell about the others.
    """
    if geometry.isNull():
        return False
    if geometry.boundingBox().isFinite():
        kind = wkb_kind(geometry.wkbType())
        if kind == 'point' and not geometry.isMultipart():
            return False
        if (
            kind == 'line'
            and not QgsWkbTypes.isCurvedType(geometry.wkbType())
            and all(part.nCoordinates() > 1 for part in parts(geometry))
        ):
            return math.isnan(geometry.length())
    return not all(finite(part) for part in parts(geometry))


def straight(geometry: QgsGeometry) -> QgsGeometry:
    """The geometry in straight segments, which is what the tool reads: a
    curve becomes a line of short segments, which QGIS does also for a
    curve type whose segments are all straight."""
    if not QgsWkbTypes.isCurvedType(geometry.wkbType()):
        return geometry
    return QgsGeometry(not_none(geometry.constGet()).segmentize())


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
    source: QgsFeatureSource | QgsAbstractFeatureSource,
    request: QgsFeatureRequest,
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


def stem(name: str) -> str:
    """The name of a layer as a file and a table name have it."""
    return re.sub(r'[^\w.-]+', '_', name) or 'layer'


def require_readable(item: Input) -> None:
    """Raises if the data of the input cannot be read."""
    if not item.valid:
        raise RunError(
            f'The layer {item.name} is not valid, its data cannot be read.'
        )
    if item.gone:
        raise RunError(item.gone)


def own_file(
    item: Input, target: QgsCoordinateReferenceSystem | None
) -> Dataset | None:
    """The file of the layer, where the tool can read the input from it as it
    is: all of it, in the target CRS."""
    if item.direct is None or (target is not None and moves(item.crs, target)):
        return None
    return item.direct


def reprojection(
    item: Input,
    target: QgsCoordinateReferenceSystem | None,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
) -> QgsCoordinateTransform | None:
    """What brings the input to the target CRS, or none where it is in it."""
    crs = item.source.sourceCrs()
    if target is None or not moves(crs, target):
        return None
    feedback.pushInfo(
        f'Reprojecting {item.name} from {crs.authid()} to {target.authid()}'
    )
    return QgsCoordinateTransform(crs, target, context.transformContext())


def geometries(
    item: Input,
    transform: QgsCoordinateTransform | None,
    feedback: QgsProcessingFeedback,
) -> c.Iterator[tuple[int, QgsGeometry]]:
    """The id and the geometry of each feature that was asked for, in the CRS
    of the transform."""
    for feature in chosen(item, feedback):
        geometry = feature.geometry()
        if transform is not None and not geometry.isNull():
            try:
                geometry.transform(transform)
            except QgsCsException as error:
                raise RunError(
                    f'Could not reproject {item.name}: {error}'
                ) from error
        yield feature.id(), geometry


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
            found = features(item.reader, request)
        else:
            request.setFilterFids(batch)
            found = sorted(features(item.reader, request), key=QgsFeature.id)
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
        for feature in features(item.reader, request)
        if item.ids is None or feature.id() in item.ids
    )


def left_out(
    item: Input,
    fid: int,
    geometry: QgsGeometry,
    reason: str,
    context: QgsProcessingContext,
) -> Skipped:
    """A feature that the tool would reject, which is left out of the run,
    or stops it where the user asked for that."""
    if aborts_on_invalid(context):
        raise RunError(f'{item.name}, feature {fid}: {reason}')
    return Skipped(fid, geometry, reason)


def write(
    writer: QgsVectorFileWriter, batch: list[QgsFeature], name: str
) -> None:
    """Writes the features of the batch to the export, and empties it."""
    if batch and not writer.addFeatures(batch):
        raise RunError(f'Could not export {name}: {writer.errorMessage()}')
    batch.clear()


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
    name = stem(item.name)
    path = directory / f'{name}.gpkg'
    index = 1
    while path.exists():
        index += 1
        path = directory / f'{name}_{index}.gpkg'
    transform = reprojection(item, target, context, feedback)
    crs = (
        item.source.sourceCrs()
        if transform is None
        else transform.destinationCrs()
    )
    options = QgsVectorFileWriter.SaveVectorOptions()
    options.driverName = 'GPKG'
    options.fileEncoding = 'UTF-8'
    options.layerName = name
    if not crs.isValid():
        # A layer without a CRS is one in no CRS, which the tool reads as
        # such, and not one in an "undefined" CRS, which it cannot parse
        options.layerOptions = ['SRID=-1']
    # The tool reads geometries, and an attribute such as `fid` can clash;
    # it reads straight segments, which curves become
    writer = QgsVectorFileWriter.create(
        str(path),
        QgsFields(),
        QgsWkbTypes.linearType(item.source.wkbType()),
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
        for fid, found in geometries(item, transform, feedback):
            geometry = straight(found)
            reason = (skip or {}).get(fid) or invalid_reason(geometry)
            if reason is not None:
                skipped.append(left_out(item, fid, geometry, reason, context))
                continue
            exported = QgsFeature()
            exported.setGeometry(geometry)
            batch.append(exported)
            ids.append(fid)
            if len(batch) == BATCH:
                write(writer, batch, item.name)
        write(writer, batch, item.name)
    finally:
        # Closes the file also when the export fails
        del writer
    return Prepared(Dataset(str(path), name), tuple(ids), tuple(skipped))


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
    require_readable(item)
    found = own_file(item, target)
    if direct and not skip and found is not None:
        return Prepared(found)
    return export_dataset(item, directory, target, context, feedback, skip)
