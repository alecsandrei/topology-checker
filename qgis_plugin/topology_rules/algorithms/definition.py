from __future__ import annotations

import dataclasses
from pathlib import Path

from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProcessingOutputNumber,
    QgsProcessingParameterFile,
    QgsProcessingParameterNumber,
    QgsVectorLayer,
)

from .._typing import Parameters
from ..core.commands import ERROR_LAYERS, rule_counts, run_validate
from ..core.toml import TomlUnavailable, loads
from ..utils import QgsProcessingException, double_type, not_none
from .base import (
    COUNT,
    TOLERANCE,
    executable_path,
    failures,
    refuse_files,
    scratch,
    tolerance_of,
)
from .sinks import SinkAlgorithm

DEFINITION = 'DEFINITION'


def errors_crs(path: str) -> QgsCoordinateReferenceSystem:
    """The CRS of the errors of a run, which the tool writes only when it
    finds some."""
    for name in ERROR_LAYERS:
        layer = QgsVectorLayer(f'{path}|layername={name}', name, 'ogr')
        if layer.isValid():
            return layer.crs()
    return QgsCoordinateReferenceSystem()


def data_files(definition: Path) -> list[str]:
    """The files that the layers of a definition are read from. A file that
    cannot be read names none: the tool says what is wrong with it."""
    try:
        config = loads(definition.read_text(encoding='utf-8'))
    except (OSError, ValueError, TomlUnavailable):
        # With no reader here the tool still reads the file, and the
        # guard goes
        return []
    layers = config.get('layers')
    if not isinstance(layers, dict):
        return []
    return [
        str(definition.parent / str(found['path']).replace('\\', '/'))
        for found in layers.values()
        if isinstance(found, dict) and found.get('path')
    ]


@dataclasses.dataclass(frozen=True)
class Plan:
    """What the main thread found out for the run."""

    executable: str
    definition: Path
    tolerance: float | None


class CheckDefinition(SinkAlgorithm):
    """Runs the rules of a topology.toml, the file that the command line
    also runs."""

    def __init__(self) -> None:
        super().__init__()
        self.plan: Plan | None = None

    def createInstance(self) -> CheckDefinition:
        return CheckDefinition()

    def name(self) -> str:
        return 'check_definition'

    def displayName(self) -> str:
        return 'Check topology definition'

    def group(self) -> str:
        return 'Utilities'

    def groupId(self) -> str:
        return 'utilities'

    def shortHelpString(self) -> str:
        return (
            'Runs the rules of a topology.toml on the data that it names. '
            'The paths in the file are relative to the file. A tolerance '
            'given here replaces the one of the file.'
        )

    def initAlgorithm(self, configuration: Parameters | None = None):
        self.addParameter(
            QgsProcessingParameterFile(
                DEFINITION, 'Topology definition', extension='toml'
            )
        )
        self.addParameter(
            QgsProcessingParameterNumber(
                TOLERANCE,
                'Tolerance (empty for the one of the file)',
                double_type(),
                optional=True,
                minValue=0,
            )
        )
        self.add_sinks()
        self.addOutput(QgsProcessingOutputNumber(COUNT, 'Number of errors'))

    def prepareAlgorithm(
        self,
        parameters: Parameters,
        context: QgsProcessingContext,
        feedback: QgsProcessingFeedback | None,
    ) -> bool:
        """Reads the file and the settings, which belong to the main thread,
        so that the run only has to use them."""

        def prepare() -> None:
            definition = Path(
                self.parameterAsFile(parameters, DEFINITION, context)
            )
            if not definition.is_file():
                raise QgsProcessingException(
                    f'The definition does not exist: {definition}'
                )
            refuse_files(
                self.sink_files(parameters, context), data_files(definition)
            )
            self.plan = Plan(
                executable_path(),
                definition,
                tolerance_of(self, parameters, context),
            )

        return self.prepare_with(prepare)

    def processAlgorithm(
        self,
        parameters: Parameters,
        context: QgsProcessingContext,
        feedback: QgsProcessingFeedback | None,
    ) -> Parameters:
        feedback = feedback or QgsProcessingFeedback()
        self.check_prepared()
        plan = not_none(self.plan)
        with (
            failures(),
            scratch() as tmp,
        ):
            path = str(Path(tmp) / 'errors.gpkg')
            counts = rule_counts(
                run_validate(
                    plan.executable,
                    str(plan.definition),
                    plan.tolerance,
                    path,
                    feedback,
                )
            )
            results = self.write_sinks(
                parameters,
                context,
                errors_crs(path),
                path,
                {},
                [],
                plan.definition.stem,
            )
        feedback.pushInfo(f'{sum(counts)} topology errors')
        return {**results, COUNT: sum(counts)}
