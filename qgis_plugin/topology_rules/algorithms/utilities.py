from __future__ import annotations

import collections.abc as c
import contextlib
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
from ..core import engine
from ..core.checks import UNREADABLE
from ..core.commands import utility_command
from ..core.inputs import Skipped, prepare
from ..core.layers import Input, require_kind
from ..core.runner import error_message, execute
from ..utils import QgsProcessingException, not_none, source_type
from .base import (
    INPUT,
    OUTPUT,
    Planned,
    failures,
    memory_copy,
    refuse_overwrite,
    release_files,
    scratch,
)

# The features of the input that the tool would reject, which are left out
SKIPPED = 'INVALID'


@dataclasses.dataclass(frozen=True)
class Plan:
    """What the main thread found out for the run."""

    tool: engine.Tool
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
            sink = self.parameterAsOutputLayer(parameters, OUTPUT, context)
            refuse_overwrite([sink], [item])
            release_files([sink])
            self.plan = Plan(engine.tool(), item)

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
            contextlib.ExitStack() as stack,
        ):
            output = str(Path(tmp) / 'output.gpkg')
            feedback.pushInfo(engine.describe(plan.tool))
            if isinstance(plan.tool, engine.Package):
                from ..core import package

                skipped = package.run_utility(
                    self.command, plan.item, output, context, feedback
                )
                report_skipped(plan.item, skipped, feedback)
            else:
                skipped = self.run_executable(
                    plan.tool, plan.item, output, context, feedback
                )
            copy = stack.enter_context(memory_copy(output))
            result = QgsVectorLayer(copy, 'output', 'ogr')
            try:
                if not result.isValid():
                    raise QgsProcessingException('The tool wrote no output')
                # The lines are in the CRS of the input, which the executable
                # may write to the file as another one
                crs = plan.item.crs if plan.item.crs.isValid() else result.crs()
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
        return {OUTPUT: destination, SKIPPED: len(skipped)}

    def run_executable(
        self,
        executable: str,
        item: Input,
        output: str,
        context: QgsProcessingContext,
        feedback: QgsProcessingFeedback,
    ) -> tuple[Skipped, ...]:
        """Runs the command of the executable, which writes the lines to the
        GeoPackage at the output, and returns the features that were left
        out."""
        # The exports have a folder of their own, where a layer named after
        # the output does not take its place
        directory = Path(output).parent / 'inputs'
        directory.mkdir()
        found = prepare(item, directory, None, context, feedback)
        completed = execute(
            utility_command(executable, self.command, found.dataset, output),
            feedback,
        )
        if (
            completed.exit_code != 0
            and found.ids is None
            and UNREADABLE.search(error_message(completed))
        ):
            # The tool cannot read the file as it is: the export leaves out
            # what it would reject
            found = prepare(
                item, directory, None, context, feedback, direct=False
            )
            completed = execute(
                utility_command(
                    executable, self.command, found.dataset, output
                ),
                feedback,
            )
        report_skipped(item, found.skipped, feedback)
        if completed.exit_code != 0:
            raise QgsProcessingException(error_message(completed))
        return found.skipped


def report_skipped(
    item: Input,
    skipped: t.Sequence[Skipped],
    feedback: QgsProcessingFeedback,
) -> None:
    """Says in the log which features were left out of the run."""
    if skipped:
        feedback.pushWarning(
            f'{len(skipped)} features of {item.name} were left out of the run'
        )
    for left_out in skipped:
        feedback.pushInfo(
            f'Left out of the run: {item.name}, feature {left_out.fid}: '
            f'{left_out.reason}'
        )


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
