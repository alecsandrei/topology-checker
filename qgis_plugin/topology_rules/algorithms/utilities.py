from __future__ import annotations

import collections.abc as c
import dataclasses
import typing as t
from pathlib import Path

from qgis.core import (
    QgsFeature,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProcessingOutputNumber,
    QgsProcessingParameterFeatureSink,
    QgsProcessingParameterVectorLayer,
    QgsProcessingUtils,
    QgsVectorLayer,
)

from .._typing import Parameters
from ..core.checks import UNREADABLE
from ..core.commands import utility_command
from ..core.inputs import prepare
from ..core.layers import Input, require_kind
from ..core.runner import error_message, execute
from ..utils import QgsProcessingException, not_none, source_type
from .base import (
    INPUT,
    OUTPUT,
    Planned,
    executable_path,
    failures,
    refuse_overwrite,
    scratch,
)

# The features of the input that the tool would reject, which are left out
SKIPPED = 'INVALID'


@dataclasses.dataclass(frozen=True)
class Plan:
    """What the main thread found out for the run."""

    executable: str
    item: Input


class UtilityAlgorithm(Planned):
    """Runs a `utilities` command that turns a line layer into another."""

    command = ''
    algorithm_name = ''
    display = ''
    help = ''

    def __init__(self) -> None:
        super().__init__()
        self.plan: Plan | None = None

    def createInstance(self) -> UtilityAlgorithm:
        return type(self)()

    def name(self) -> str:
        return self.algorithm_name

    def displayName(self) -> str:
        return self.display

    def group(self) -> str:
        return 'Utilities'

    def groupId(self) -> str:
        return 'utilities'

    def shortHelpString(self) -> str:
        return self.help

    def initAlgorithm(self, configuration: Parameters | None = None):
        self.addParameter(
            QgsProcessingParameterVectorLayer(
                INPUT, 'Input layer', [source_type('VectorLine')]
            )
        )
        self.addParameter(
            QgsProcessingParameterFeatureSink(
                OUTPUT, 'Output', source_type('VectorLine')
            )
        )
        self.addOutput(
            QgsProcessingOutputNumber(SKIPPED, 'Features left out of the run')
        )

    def prepareAlgorithm(
        self,
        parameters: Parameters,
        context: QgsProcessingContext,
        feedback: QgsProcessingFeedback | None,
    ) -> bool:
        """Reads the layer and the settings, which belong to the main thread,
        so that the run only has to use them."""

        def prepare() -> None:
            layer = self.parameterAsVectorLayer(parameters, INPUT, context)
            source = None
            if layer is not None:
                source = QgsProcessingUtils.variantToSource(layer.id(), context)
            if layer is None or source is None:
                raise QgsProcessingException(
                    self.invalidSourceError(parameters, INPUT)
                )
            item = Input(layer.name(), source, layer)
            require_kind(item, 'line')
            refuse_overwrite(
                [self.parameterAsOutputLayer(parameters, OUTPUT, context)],
                [item],
            )
            self.plan = Plan(executable_path(), item)

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
            output = str(Path(tmp) / 'output.gpkg')
            found = prepare(plan.item, Path(tmp), None, context, feedback)
            command = utility_command(
                plan.executable, self.command, found.dataset, output
            )
            completed = execute(command, feedback)
            if (
                completed.exit_code != 0
                and found.ids is None
                and UNREADABLE.search(error_message(completed))
            ):
                # The tool cannot read the file as it is: the export leaves
                # out what it would reject
                found = prepare(
                    plan.item, Path(tmp), None, context, feedback, direct=False
                )
                completed = execute(
                    utility_command(
                        plan.executable, self.command, found.dataset, output
                    ),
                    feedback,
                )
            if found.skipped:
                feedback.pushWarning(
                    f'{len(found.skipped)} features of {plan.item.name} '
                    'were left out of the run'
                )
            for left_out in found.skipped:
                feedback.pushInfo(
                    f'Left out of the run: {plan.item.name}, feature '
                    f'{left_out.fid}: {left_out.reason}'
                )
            if completed.exit_code != 0:
                raise QgsProcessingException(error_message(completed))
            result = QgsVectorLayer(output, 'output', 'ogr')
            try:
                if not result.isValid():
                    raise QgsProcessingException('The tool wrote no output')
                crs = result.crs() if result.crs().isValid() else plan.item.crs
                sink, destination = self.parameterAsSink(
                    parameters,
                    OUTPUT,
                    context,
                    result.fields(),
                    result.wkbType(),
                    crs,
                )
                if sink is None:
                    raise QgsProcessingException(
                        self.invalidSinkError(parameters, OUTPUT)
                    )
                for feature in t.cast(
                    'c.Iterable[QgsFeature]', result.getFeatures()
                ):
                    sink.addFeature(feature)
                del sink
            finally:
                # Closed before the folder of the file goes, also on failure
                del result
        return {OUTPUT: destination, SKIPPED: len(found.skipped)}


class MergeLines(UtilityAlgorithm):
    command = 'merge-lines'
    algorithm_name = 'merge_lines'
    display = 'Merge lines'
    help = 'Join lines at endpoints shared by exactly two lines.'


class ExplodeLines(UtilityAlgorithm):
    command = 'explode-lines'
    algorithm_name = 'explode_lines'
    display = 'Explode lines'
    help = 'Split lines into single segments.'
