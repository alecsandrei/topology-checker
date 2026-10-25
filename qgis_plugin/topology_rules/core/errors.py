"""The errors of a run as both engines find them: the rows of the layer of
each family of geometry, which the log and the panel hear of as each rule
is done, and which are written to the GeoPackage right after. The panel
hears of each error with the id that it gets in its layer."""

from __future__ import annotations

import collections
import collections.abc as c
import dataclasses
import itertools
import typing as t
from pathlib import Path

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
from .progress import Checked, Found, checked, left_out, log_rule

if t.TYPE_CHECKING:
    from .checks import Job, Outcome

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
    """The number of errors of each rule that is done, what the engine
    said about the data, and how many errors of each family were written."""

    counts: list[int]
    warnings: list[str]
    written: dict[str, int] = dataclasses.field(default_factory=dict)


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
        # data() is bytes at run time; some stubs type it as str
        found.append(
            Found(family, t.cast('bytes', member.asWkb().data()), values)
        )
    return found


def remove_file(path: str) -> None:
    """Removes the GeoPackage at the path, if there is one."""
    try:
        for suffix in ('', '-wal', '-shm'):
            Path(f'{path}{suffix}').unlink(missing_ok=True)
    except OSError as error:
        raise RunError(
            tr('Could not remove {path}: {error}').format(
                path=path, error=error
            )
        ) from error


class ErrorsFile:
    """The GeoPackage that the errors are written to as each rule is done:
    the layers `errors_line`, `errors_point` and `errors_polygon`, those
    that have any. A run that finds nothing writes nothing.

    The errors of a layer have the ids 1, 2, 3 and so on in the order they
    are written. Written tells how many of each family the file has already,
    which more errors follow.
    """

    def __init__(
        self,
        path: str,
        crs: QgsCoordinateReferenceSystem,
        context: QgsProcessingContext,
        written: c.Mapping[str, int] | None = None,
    ) -> None:
        self.path = path
        self.crs = crs
        self.context = context
        self.fields = QgsVectorLayer(
            f'None?field={COLUMNS}', 'errors', 'memory'
        ).fields()
        # The families that have a layer
        self.made = {name for name, count in (written or {}).items() if count}
        if not self.made:
            # What an earlier attempt of the run wrote is not of this one
            remove_file(path)

    def add(self, found: c.Sequence[Found]) -> None:
        """Writes the errors of a rule, which are in the order of their
        families, after those of the rules before it."""
        Action = QgsVectorFileWriter.ActionOnExistingFile
        for family, items in itertools.groupby(found, lambda f: f.family):
            name = f'errors_{family}'
            action = (
                Action.AppendToLayerNoNewFields
                if family in self.made
                else Action.CreateOrOverwriteLayer
                if self.made
                else Action.CreateOrOverwriteFile
            )
            writer = create_layer(
                self.path,
                name,
                self.fields,
                MULTI[family],
                self.crs,
                action,
                self.context,
            )
            self.made.add(family)
            try:
                write_found(writer, items, self.fields, name)
            finally:
                del writer


class Gathered:
    """The errors of the rules that are done. The log, the progress and the
    panel hear of each rule as soon as it is done, in the order of the
    rules, and then its errors are written to the file: none are kept."""

    def __init__(
        self,
        jobs: t.Sequence[Job],
        layers: c.Mapping[str, str],
        feedback: QgsProcessingFeedback,
        file: ErrorsFile,
    ) -> None:
        self.jobs = jobs
        self.layers = layers
        self.feedback = feedback
        self.file = file
        self.counts: list[int] = []
        # How many errors of each family the rules that are done found
        self.written: collections.Counter[str] = collections.Counter()

    def add(
        self, found: c.Sequence[Found], count: int, more: bool = False
    ) -> None:
        """A rule is done: the rows of its errors, and how many it found.
        With more, the next rule is done too."""
        self.counts.append(count)
        index = len(self.counts) - 1
        log_rule(self.feedback, self.jobs, index, count)
        # The errors in the order the panel reads them from the layers: by
        # family, then as they were found
        ordered = tuple(
            sorted(found, key=lambda item: FAMILIES.index(item.family))
        )
        done = Checked(
            index,
            len(self.jobs),
            count,
            ordered,
            self.layers,
            dict(self.written),
        )
        self.written.update(item.family for item in ordered)
        checked(self.feedback, done, more)
        self.write(ordered)

    def write(self, found: c.Sequence[Found]) -> None:
        self.file.add(found)

    def errors(self, warnings: c.Iterable[str] = ()) -> Errors:
        return Errors(self.counts, list(warnings), dict(self.written))


def add_invalid(
    path: str,
    outcome: Outcome,
    total: int,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
) -> None:
    """Writes the features that were left out of a run after the errors of
    its rules, by family, and tells the feedback of them as errors of their
    own, with the ids they got."""
    found = tuple(
        sorted(
            (item.found() for item in outcome.invalid),
            key=lambda item: FAMILIES.index(item.family),
        )
    )
    if not found:
        return
    file = ErrorsFile(path, outcome.crs, context, outcome.written)
    file.add(found)
    left_out(
        feedback,
        Checked(
            len(outcome.counts),
            total,
            len(found),
            found,
            outcome.layers,
            outcome.written,
        ),
    )


def create_layer(
    path: str,
    name: str,
    fields: QgsFields,
    kind: Qgis.WkbType,
    crs: QgsCoordinateReferenceSystem,
    action: QgsVectorFileWriter.ActionOnExistingFile,
    context: QgsProcessingContext,
) -> QgsVectorFileWriter:
    """A layer of the GeoPackage at the path: a new file, a new layer of the
    file, or more features of a layer that is there, as the action says."""
    options = QgsVectorFileWriter.SaveVectorOptions()
    options.driverName = 'GPKG'
    options.fileEncoding = 'UTF-8'
    options.layerName = name
    options.actionOnExistingFile = action
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


def write_found(
    writer: QgsVectorFileWriter,
    found: c.Iterable[Found],
    fields: QgsFields,
    name: str,
) -> None:
    """Writes the errors, whose geometries are multi geometries already."""
    batch: list[QgsFeature] = []
    for item in found:
        feature = QgsFeature(fields)
        feature.setGeometry(from_wkb(item.wkb))
        feature.setAttributes(list(item.values))
        batch.append(feature)
        if len(batch) == BATCH:
            write(writer, batch, name)
    write(writer, batch, name)
