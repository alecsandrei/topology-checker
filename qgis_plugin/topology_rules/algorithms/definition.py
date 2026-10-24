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
from ..core.commands import ERROR_LAYERS, definition_command, rule_counts
from ..core.engine import describe
from ..core.runner import execute
from ..core.toml import TomlUnavailable, loads
from ..utils import QgsProcessingException, double_type, not_none, tr
from .base import (
    COUNT,
    TOLERANCE,
    executable_path,
    failures,
    memory_copy,
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
        return tr('Check topology definition')

    def group(self) -> str:
        return tr('Utilities')

    def groupId(self) -> str:
        return 'utilities'

    def shortHelpString(self) -> str:
        return tr(
            'Runs the rules of a topology.toml on the data that it names. '
            'The paths in the file are relative to the file. A tolerance '
            'given here replaces the one of the file. The topology-checker '
            'executable runs the file, as on the command line.'
        )

    def initAlgorithm(self, configuration: Parameters | None = None):
        self.addParameter(
            QgsProcessingParameterFile(
                DEFINITION, tr('Topology definition'), extension='toml'
            )
        )
        self.addParameter(
            QgsProcessingParameterNumber(
                TOLERANCE,
                tr('Tolerance (empty for the one of the file)'),
                double_type(),
                optional=True,
                minValue=0,
            )
        )
        self.add_sinks()
        self.addOutput(QgsProcessingOutputNumber(COUNT, tr('Number of errors')))

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
                    tr('The definition does not exist: {path}').format(
                        path=definition
                    )
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
            feedback.pushInfo(describe(plan.executable))
            counts = rule_counts(
                execute(
                    definition_command(
                        plan.executable,
                        str(plan.definition),
                        plan.tolerance,
                        path,
                    ),
                    feedback,
                )
            )
            with memory_copy(path) as source:
                results = self.write_sinks(
                    parameters,
                    context,
                    errors_crs(source),
                    source,
                    [],
                    plan.definition.stem,
                )
        feedback.pushInfo(
            tr('{count} topology errors').format(count=sum(counts))
        )
        return {**results, COUNT: sum(counts)}
