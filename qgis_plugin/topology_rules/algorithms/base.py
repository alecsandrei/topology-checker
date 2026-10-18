from __future__ import annotations

import collections.abc as c
import contextlib
import tempfile
from pathlib import Path

from qgis.core import (
    NULL,
    QgsProcessingAlgorithm,
    QgsProcessingContext,
)
from qgis.PyQt.QtGui import QIcon

from .._typing import Parameters
from ..core import binary
from ..core.layers import Input, same_file
from ..enums import Asset
from ..exceptions import ExecutableError, RunError
from ..utils import QgsProcessingException

INPUT = 'INPUT'
OTHER = 'OTHER'
TOLERANCE = 'TOLERANCE'
OUTPUT = 'OUTPUT'
COUNT = 'COUNT'
# The documentation, which the help of an algorithm links to
DOCS = 'https://alecsandrei.github.io/topology-checker/stable/'


def scratch() -> tempfile.TemporaryDirectory[str]:
    """A folder for the files of one run, which goes with the run."""
    return tempfile.TemporaryDirectory(prefix='topology-checker-')


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


def refuse_overwrite(
    outputs: c.Iterable[str], inputs: c.Iterable[Input | None]
) -> None:
    """Stops a run that would replace the file of a layer it checks."""
    files = [item.path for item in inputs if item is not None]
    for output in outputs:
        for path in files:
            if path is not None and same_file(output, path):
                raise QgsProcessingException(
                    f'The output {output} is a file of an input layer, which '
                    'the run would replace. Choose another output.'
                )


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
        return True

    def check_prepared(self) -> None:
        if self.failure is not None:
            raise self.failure
