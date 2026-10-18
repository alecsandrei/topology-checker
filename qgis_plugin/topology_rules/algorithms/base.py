from __future__ import annotations

import collections.abc as c
import contextlib
import tempfile
from pathlib import Path

from qgis.core import (
    NULL,
    QgsProcessingAlgorithm,
    QgsProcessingContext,
    QgsProject,
    QgsVectorLayer,
)
from qgis.PyQt.QtGui import QIcon

from .._typing import Parameters
from ..core import binary
from ..core.layers import Input, same_file, uri_parts
from ..enums import Asset
from ..exceptions import ExecutableError, RunError
from ..utils import QgsProcessingException, not_none

INPUT = 'INPUT'
OTHER = 'OTHER'
TOLERANCE = 'TOLERANCE'
OUTPUT = 'OUTPUT'
COUNT = 'COUNT'
# The documentation, which the help of an algorithm links to
# How many other names are tried for a file that is in use
MAX_NAMES = 20
DOCS = 'https://alecsandrei.github.io/topology-checker/stable/'


def scratch() -> tempfile.TemporaryDirectory[str]:
    """A folder for the files of one run, which goes with the run."""
    # A file of the folder that is still open, as on Windows, is left behind
    # and not an error of the run
    return tempfile.TemporaryDirectory(
        prefix='topology-checker-', ignore_cleanup_errors=True
    )


@contextlib.contextmanager
def failures() -> c.Iterator[None]:
    """Turns the errors of the plugin into the exception of Processing."""
    try:
        yield
    except (ExecutableError, RunError) as error:
        raise QgsProcessingException(str(error)) from error


def executable_path() -> str:
    """The executable, which raises an ExecutableError without one."""
    return str(binary.find_executable())


def tolerance_of(
    algorithm: QgsProcessingAlgorithm,
    parameters: Parameters,
    context: QgsProcessingContext,
) -> float | None:
    """The tolerance that was given, or none for the default of the CRS."""
    value = parameters.get(TOLERANCE)
    if value is None or value == '' or value == NULL:
        return None
    return algorithm.parameterAsDouble(parameters, TOLERANCE, context)


def geopackage_name(output: str) -> str:
    """The errors go to a GeoPackage, whatever name was given."""
    path = Path(output)
    if path.suffix.lower() == '.gpkg':
        return output
    if path.suffix:
        return str(path.with_suffix('.gpkg'))
    return f'{output}.gpkg'


def refuse_files(outputs: c.Iterable[str], files: c.Iterable[str]) -> None:
    """Stops a run that would replace one of the files it reads."""
    files = list(files)
    for output in outputs:
        for path in files:
            if same_file(output, path):
                raise QgsProcessingException(
                    f'The output {output} is a file of an input layer, which '
                    'the run would replace. Choose another output.'
                )


def refuse_overwrite(
    outputs: c.Iterable[str], inputs: c.Iterable[Input | None]
) -> None:
    """Stops a run that would replace the file of a layer it checks."""
    refuse_files(
        outputs,
        (item.path for item in inputs if item is not None and item.path),
    )


def release_files(files: c.Iterable[str]) -> None:
    """Takes the layers of the project that read one of the files out of it,
    so that a run can write the file again: where a file is open, as on
    Windows, it cannot be replaced. Main thread only."""
    files = list(files)
    project = not_none(QgsProject.instance())
    for layer in list(project.mapLayers().values()):
        if not isinstance(layer, QgsVectorLayer):
            continue
        if layer.providerType() != 'ogr':
            continue
        where = str(uri_parts(layer).get('path') or '')
        if where and any(same_file(where, path) for path in files):
            project.removeMapLayer(layer.id())


def free_file(path: str) -> str:
    """The path to write to: the one given if what is there can be removed,
    else a new one beside it, so that a run does not fail for a file that
    something else has open."""
    candidate, index = Path(path), 1
    while True:
        try:
            for suffix in ('', '-wal', '-shm'):
                Path(f'{candidate}{suffix}').unlink(missing_ok=True)
            return str(candidate)
        except OSError:
            index += 1
            candidate = Path(path).with_name(
                f'{Path(path).stem}_{index}{Path(path).suffix}'
            )
            if index > MAX_NAMES:
                raise QgsProcessingException(
                    f'The file {path} cannot be replaced: it is in use. '
                    'Choose another output.'
                ) from None


class Planned(QgsProcessingAlgorithm):
    """An algorithm that finds out what it needs on the main thread, where the
    layers and the settings are, and runs in another thread on that.

    Processing never reports a task whose preparation failed, so a failure
    there is kept and raised as the run starts, which it does report.
    """

    def __init__(self) -> None:
        super().__init__()
        self.failure: Exception | None = None

    def icon(self) -> QIcon:
        return QIcon(Asset.ICON.value.as_posix())

    def helpUrl(self) -> str:
        return DOCS

    def tags(self) -> list[str]:
        return ['topology']

    def prepare_with(self, action: c.Callable[[], None]) -> bool:
        self.failure = None
        try:
            with failures():
                action()
        except QgsProcessingException as error:
            self.failure = error
        except Exception as error:
            # Whatever else went wrong is reported, not dropped by Processing
            self.failure = QgsProcessingException(
                str(error) or type(error).__name__
            )
        return True

    def check_prepared(self) -> None:
        if self.failure is not None:
            raise self.failure
