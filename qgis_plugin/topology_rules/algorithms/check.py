from __future__ import annotations

import dataclasses
import json
import typing as t
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsCoordinateReferenceSystem,
    QgsCoordinateTransform,
    QgsProcessingAlgorithm,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProcessingOutputNumber,
    QgsProcessingOutputString,
    QgsProcessingParameterFileDestination,
    QgsProcessingParameterNumber,
    QgsProcessingParameterString,
    QgsProcessingUtils,
    QgsRectangle,
    QgsVectorLayer,
)

from .._typing import Parameters
from ..core import binary
from ..core.checks import Job, renumber_errors, run_jobs
from ..core.invalid import append_invalid
from ..core.layers import Input, require_kind
from ..utils import QgsProcessingException, double_type, named, not_none
from .base import (
    COUNT,
    OUTPUT,
    TOLERANCE,
    Planned,
    executable_path,
    failures,
    geopackage_name,
    refuse_overwrite,
    scratch,
    tolerance_of,
)

RULES = 'RULES'
# Which part of the layers to check: selected features, the visible area
SCOPE = 'SCOPE'
COUNTS = 'COUNTS'
LAYERS = 'LAYERS'
INVALID = 'INVALID'


def hidden_flags() -> t.Any:
    scoped = getattr(Qgis, 'ProcessingAlgorithmFlag', None)
    if scoped is not None:
        return scoped.HideFromToolbox | scoped.HideFromModeler
    # QGIS 3 names the flags on the class
    return named(QgsProcessingAlgorithm, 'FlagHideFromToolbox') | named(
        QgsProcessingAlgorithm, 'FlagHideFromModeler'
    )


@dataclasses.dataclass(frozen=True)
class Plan:
    """What the main thread found out for the run."""

    executable: str
    jobs: list[Job]
    tolerance: float | None
    output: str


class CheckRulesAlgorithm(Planned):
    """Runs several rules on project layers in one pass of the executable.

    The rules are a JSON list of {"rule": name, "layer": id, "other": id or
    null}, which the dock builds, so the algorithm is not in the toolbox. The
    errors go to one GeoPackage, whose features name the features of the
    layers by their ids.
    """

    def __init__(self) -> None:
        super().__init__()
        self.plan: Plan | None = None

    def createInstance(self) -> CheckRulesAlgorithm:
        return CheckRulesAlgorithm()

    def name(self) -> str:
        return 'check_rules'

    def displayName(self) -> str:
        return 'Check rules'

    def group(self) -> str:
        return 'Utilities'

    def groupId(self) -> str:
        return 'utilities'

    def flags(self) -> t.Any:
        return super().flags() | hidden_flags()

    def shortHelpString(self) -> str:
        return 'Runs several topology rules on layers in one pass.'

    def initAlgorithm(self, configuration: Parameters | None = None):
        self.addParameter(
            QgsProcessingParameterString(RULES, 'Rules', multiLine=True)
        )
        self.addParameter(
            QgsProcessingParameterString(SCOPE, 'Scope', optional=True)
        )
        self.addParameter(
            QgsProcessingParameterNumber(
                TOLERANCE,
                'Tolerance in layer units (empty for the default of the CRS)',
                double_type(),
                optional=True,
                minValue=0,
            )
        )
        self.addParameter(
            QgsProcessingParameterFileDestination(
                OUTPUT, 'Errors', 'GeoPackage (*.gpkg)'
            )
        )
        self.addOutput(QgsProcessingOutputNumber(COUNT, 'Number of errors'))
        self.addOutput(
            QgsProcessingOutputString(COUNTS, 'Errors of each rule as JSON')
        )
        self.addOutput(
            QgsProcessingOutputString(LAYERS, 'Layer ids by name as JSON')
        )
        self.addOutput(QgsProcessingOutputNumber(INVALID, 'Invalid features'))

    def prepareAlgorithm(
        self,
        parameters: Parameters,
        context: QgsProcessingContext,
        feedback: QgsProcessingFeedback | None,
    ) -> bool:
        """Reads the rules, the layers and the settings, which belong to the
        main thread, so that the run only has to use them."""

        def prepare() -> None:
            executable = executable_path()
            known = {rule.name: rule for rule in binary.rules()}
            requested = self.parameterAsFileOutput(parameters, OUTPUT, context)
            output = geopackage_name(requested)
            if output != requested and feedback is not None:
                feedback.pushInfo(f'The errors are written to {output}')
            scope = self.read_scope(parameters, context)
            inputs: dict[str, Input] = {}
            jobs = []
            for entry in self.read_entries(
                self.parameterAsString(parameters, RULES, context)
            ):
                rule = known.get(entry['rule'])
                if rule is None:
                    raise QgsProcessingException(
                        f'Unknown rule: {entry["rule"]}'
                    )
                other = entry.get('other')
                if (other is None) != (rule.other is None):
                    raise QgsProcessingException(
                        f'Wrong number of layers for {rule.name}'
                    )
                # The scope limits the layer a rule checks, not the layer it
                # checks it against
                first = self.read(
                    entry['layer'], rule.layer, inputs, context, scope
                )
                second = None
                if other is not None and rule.other is not None:
                    second = self.read(
                        other, rule.other, inputs, context, {}, 'other'
                    )
                jobs.append(Job(rule, first, second))
            refuse_overwrite([requested, output], inputs.values())
            self.plan = Plan(
                executable,
                jobs,
                tolerance_of(self, parameters, context),
                output,
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
        with failures():
            # A run that finds nothing writes nothing: the file of an earlier
            # run would be taken for its errors
            for suffix in ('', '-wal', '-shm'):
                Path(plan.output + suffix).unlink(missing_ok=True)
            with scratch() as tmp:
                outcome = run_jobs(
                    plan.jobs,
                    plan.executable,
                    plan.tolerance,
                    plan.output,
                    Path(tmp),
                    context,
                    feedback,
                )
            renumber_errors(plan.output, outcome.ids)
            append_invalid(plan.output, outcome.invalid, outcome.crs, context)
        total = sum(outcome.counts) + len(outcome.invalid)
        feedback.pushInfo(f'{total} topology errors')
        return {
            OUTPUT: plan.output,
            COUNT: total,
            COUNTS: json.dumps(outcome.counts),
            LAYERS: json.dumps(outcome.layers),
            INVALID: len(outcome.invalid),
        }

    def read_scope(
        self, parameters: Parameters, context: QgsProcessingContext
    ) -> Parameters:
        text = self.parameterAsString(parameters, SCOPE, context)
        try:
            scope = json.loads(text) if text else {}
        except ValueError as error:
            raise QgsProcessingException(f'Invalid scope: {error}') from error
        return scope if isinstance(scope, dict) else {}

    @staticmethod
    def area_in(
        extent: dict[str, t.Any],
        layer: QgsVectorLayer,
        context: QgsProcessingContext,
    ) -> QgsRectangle:
        """The area of the scope in the CRS of the layer."""
        area = QgsRectangle(*(float(v) for v in extent['rect']))
        crs = QgsCoordinateReferenceSystem()
        crs.createFromWkt(str(extent.get('crs', '')))
        if crs.isValid() and layer.crs().isValid() and crs != layer.crs():
            area = QgsCoordinateTransform(
                crs, layer.crs(), context.transformContext()
            ).transformBoundingBox(area)
        return area

    def read_entries(self, text: str) -> list[dict[str, t.Any]]:
        try:
            entries = json.loads(text)
            if not entries or not all(
                isinstance(entry, dict)
                and isinstance(entry['rule'], str)
                and isinstance(entry['layer'], str)
                for entry in entries
            ):
                raise ValueError('Expected a list of rules')
        except (ValueError, KeyError, TypeError) as error:
            raise QgsProcessingException(f'Invalid rules: {error}') from error
        return t.cast('list[dict[str, t.Any]]', entries)

    def read(
        self,
        layer_id: str,
        kind: str,
        inputs: dict[str, Input],
        context: QgsProcessingContext,
        scope: dict[str, t.Any],
        role: str = 'layer',
    ) -> Input:
        """The layer of the project with the id, of the kind of geometry,
        and the part of it that the scope asks for. A layer that is the
        layer of a rule and the other layer of another is two inputs."""
        key = f'{role}:{layer_id}'
        if key not in inputs:
            layer = QgsProcessingUtils.mapLayerFromString(layer_id, context)
            try:
                source = QgsProcessingUtils.variantToSource(layer_id, context)
            except Exception as error:
                # QGIS raises a bare exception for a layer it cannot read
                name = layer.name() if layer is not None else layer_id
                raise QgsProcessingException(
                    f'QGIS cannot read the layer {name}; it may contain '
                    f'infinite coordinates ({error})'
                ) from error
            if not isinstance(layer, QgsVectorLayer) or source is None:
                raise QgsProcessingException(f'The layer is gone: {layer_id}')
            ids, area = None, None
            if scope.get('selected') is not None:
                ids = frozenset(
                    int(i) for i in scope['selected'].get(layer_id, [])
                )
            if scope.get('extent'):
                area = self.area_in(scope['extent'], layer, context)
            inputs[key] = Input(
                layer.name(),
                source,
                layer,
                whole=ids is None and area is None,
                ids=ids,
                area=area,
            )
        require_kind(inputs[key], kind)
        return inputs[key]
