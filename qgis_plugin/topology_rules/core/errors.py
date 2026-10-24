"""The errors of a run as both engines find them: the rows of the layer of
each family of geometry, which the log and the panel hear of as each rule
is done, and the GeoPackage that they are written to."""

from __future__ import annotations

import collections.abc as c
import dataclasses
import typing as t

from qgis.core import (
    Qgis,
    QgsCoordinateReferenceSystem,
    QgsFeature,
    QgsFields,
    QgsGeometry,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsVectorFileWriter,
    QgsVectorLayer,
    QgsWkbTypes,
)

from ..exceptions import RunError
from ..utils import tr
from .commands import ERROR_LAYERS
from .inputs import BATCH, write
from .invalid import FIELDS
from .layers import wkb_kind
from .progress import Checked, Found, checked, log_rule

if t.TYPE_CHECKING:
    from .checks import Job

# The geometry of each layer of errors
MULTI = {
    'point': Qgis.WkbType.MultiPoint,
    'line': Qgis.WkbType.MultiLineString,
    'polygon': Qgis.WkbType.MultiPolygon,
}
# The id of "Undefined geographic SRS", which the executable gives a layer
# without a CRS
NO_CRS = 'SRID=0'
# The columns of an error as the executable writes them: texts of any
# length and 64-bit integers
COLUMNS = FIELDS.replace(':string', ':string(0)').replace(':integer', ':long')
# The families of the error layers, in the order the panel reads them
FAMILIES = tuple(name.removeprefix('errors_') for name in ERROR_LAYERS)


@dataclasses.dataclass
class Errors:
    """The errors of a run, as the rows of the layer of each family of
    geometry, the number of errors of each rule that is done, and what the
    engine said about the data."""

    rows: dict[str, list[Found]]
    counts: list[int]
    warnings: list[str]


def from_wkb(wkb: bytes) -> QgsGeometry:
    geometry = QgsGeometry()
    geometry.fromWkb(wkb)
    return geometry


def members(geometry: QgsGeometry) -> c.Iterator[QgsGeometry]:
    """The geometry, or each member of a collection, which goes to the layer
    of its family."""
    collection = Qgis.WkbType.GeometryCollection
    if QgsWkbTypes.flatType(geometry.wkbType()) == collection:
        for part in geometry.asGeometryCollection():
            yield from members(part)
    else:
        yield geometry


def found_in(geometry: QgsGeometry, values: tuple[object, ...]) -> list[Found]:
    """The rows of an error: one for each member, as a multi geometry, with
    the columns of the error."""
    found: list[Found] = []
    for member in members(geometry):
        family = wkb_kind(member.wkbType())
        member.convertToMultiType()
        found.append(Found(family, member, values))
    return found


class Gathered:
    """The errors of the rules that are done. The log, the progress and the
    panel hear of each rule as soon as it is done, in the order of the
    rules."""

    def __init__(
        self,
        jobs: t.Sequence[Job],
        layers: c.Mapping[str, str],
        feedback: QgsProcessingFeedback,
    ) -> None:
        self.jobs = jobs
        self.layers = layers
        self.feedback = feedback
        self.rows: dict[str, list[Found]] = {}
        self.counts: list[int] = []

    def add(self, found: c.Sequence[Found], count: int) -> None:
        """A rule is done: the rows of its errors, and how many it found."""
        for item in found:
            self.rows.setdefault(item.family, []).append(item)
        self.counts.append(count)
        index = len(self.counts) - 1
        log_rule(self.feedback, self.jobs, index, count)
        # The errors in the order the panel reads them from the layers: by
        # family, then as they were found
        ordered = sorted(found, key=lambda item: FAMILIES.index(item.family))
        checked(
            self.feedback,
            Checked(index, len(self.jobs), count, tuple(ordered), self.layers),
        )

    def errors(self, warnings: c.Iterable[str] = ()) -> Errors:
        return Errors(self.rows, self.counts, list(warnings))


def create_layer(
    path: str,
    name: str,
    fields: QgsFields,
    kind: Qgis.WkbType,
    crs: QgsCoordinateReferenceSystem,
    first: bool,
    context: QgsProcessingContext,
) -> QgsVectorFileWriter:
    """A new layer of the GeoPackage at the path, which the first one
    creates."""
    options = QgsVectorFileWriter.SaveVectorOptions()
    options.driverName = 'GPKG'
    options.fileEncoding = 'UTF-8'
    options.layerName = name
    options.actionOnExistingFile = (
        QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteFile
        if first
        else QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteLayer
    )
    if not crs.isValid():
        options.layerOptions = [NO_CRS]
    writer = QgsVectorFileWriter.create(
        path, fields, kind, crs, context.transformContext(), options
    )
    if writer is None or (
        writer.hasError() != QgsVectorFileWriter.WriterError.NoError
    ):
        message = '' if writer is None else writer.errorMessage()
        raise RunError(
            tr('Could not write {layer} to {path}: {error}').format(
                layer=name, path=path, error=message
            )
        )
    return writer


def write_rows(
    writer: QgsVectorFileWriter,
    rows: c.Iterable[tuple[QgsGeometry, c.Sequence[object]]],
    fields: QgsFields,
    name: str,
) -> None:
    """Writes the geometries, as multi geometries, with their columns."""
    batch: list[QgsFeature] = []
    for geometry, values in rows:
        geometry.convertToMultiType()
        feature = QgsFeature(fields)
        feature.setGeometry(geometry)
        feature.setAttributes(list(values))
        batch.append(feature)
        if len(batch) == BATCH:
            write(writer, batch, name)
    write(writer, batch, name)


def write_errors(
    path: str,
    errors: Errors,
    crs: QgsCoordinateReferenceSystem,
    context: QgsProcessingContext,
) -> None:
    """Writes the errors to the layers `errors_line`, `errors_point` and
    `errors_polygon` of the GeoPackage, those that have any."""
    columns = QgsVectorLayer(f'None?field={COLUMNS}', 'errors', 'memory')
    fields = columns.fields()
    for index, family in enumerate(sorted(errors.rows)):
        name = f'errors_{family}'
        writer = create_layer(
            path, name, fields, MULTI[family], crs, index == 0, context
        )
        try:
            write_rows(
                writer,
                (
                    (found.geometry, found.values)
                    for found in errors.rows[family]
                ),
                fields,
                name,
            )
        finally:
            del writer
