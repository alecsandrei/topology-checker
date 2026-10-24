from __future__ import annotations

import dataclasses
from pathlib import Path

from qgis.core import (
    QgsProcessingContext,
    QgsProcessingFeatureSourceDefinition,
    QgsProcessingFeedback,
    QgsProcessingOutputNumber,
    QgsProcessingParameterDistance,
    QgsProcessingParameterFeatureSource,
)

from .._typing import Parameters
from ..core import engine
from ..core.binary import Rule
from ..core.checks import Job, run_jobs
from ..core.layers import Input, require_kind
from ..core.style import ErrorStyle
from ..enums import Group
from ..utils import QgsProcessingException, not_none, source_type, tr
from .base import (
    COUNT,
    INPUT,
    OTHER,
    OUTPUT,
    TOLERANCE,
    failures,
    memory_copy,
    refuse_overwrite,
    release_files,
    scratch,
    tolerance_of,
)
from .sinks import SinkAlgorithm

# Words of a rule name that say nothing about what it checks
FILLER = {'must', 'not', 'be', 'have', 'by', 'with', 'of'}
GEOMETRY_WORDS = {
    'point': 'points',
    'line': 'lines',
    'polygon': 'polygons',
    'geometry': 'geometries',
}
HELP_URL = 'https://alecsandrei.github.io/topology-checker/stable/rules/'

__all__ = ['COUNT', 'INPUT', 'OTHER', 'OUTPUT', 'TOLERANCE', 'RuleAlgorithm']

_KINDS = {
    'point': ['VectorPoint'],
    'line': ['VectorLine'],
    'polygon': ['VectorPolygon'],
    'any': ['VectorAnyGeometry'],
}


def layer_types(kind: str) -> list[int]:
    return [source_type(name) for name in _KINDS.get(kind, _KINDS['any'])]


@dataclasses.dataclass(frozen=True)
class Plan:
    """What the main thread found out for the run."""

    tool: engine.Tool
    first: Input
    other: Input | None
    tolerance: float | None


class RuleAlgorithm(SinkAlgorithm):
    """Runs one topology rule on one or two layers."""

    def __init__(self, rule: Rule):
        super().__init__()
        self.rule = rule
        self.plan: Plan | None = None
        # The styles of the layers to load, which Processing owns but
        # Python must not drop
        self.styles: list[ErrorStyle] = []

    def createInstance(self) -> RuleAlgorithm:
        return RuleAlgorithm(self.rule)

    def name(self) -> str:
        return self.rule.slug

    def displayName(self) -> str:
        return self.rule.display_name

    def group(self) -> str:
        return {
            Group.POINT: tr('Points'),
            Group.LINE: tr('Lines'),
            Group.POLYGON: tr('Polygons'),
            Group.GEOMETRY: tr('All geometries'),
        }[self.rule.group]

    def groupId(self) -> str:
        return self.rule.group.value

    def shortHelpString(self) -> str:
        return tr(
            '{description}\n\nTakes {layers}. The tolerance is in the units '
            'of the input layer; leave it empty for the default of its CRS.'
        ).format(description=self.rule.description, layers=self.needs())

    def helpUrl(self) -> str:
        return HELP_URL

    def tags(self) -> list[str]:
        """The words to find the rule by: topology, the geometry and the
        words of its name that say what it checks."""
        group, _, rest = self.rule.name.partition('/')
        words = [word for word in rest.split('-') if word not in FILLER]
        return ['topology', GEOMETRY_WORDS.get(group, group), *words]

    def needs(self) -> str:
        """The layers that the rule takes, as the help says it."""

        def layer(kind: str) -> str:
            return {
                'point': tr('point layer'),
                'line': tr('line layer'),
                'polygon': tr('polygon layer'),
            }.get(kind, tr('layer'))

        if self.rule.other is None:
            return tr('one {layer}').format(layer=layer(self.rule.layer))
        return tr('a {layer} and a {other}').format(
            layer=layer(self.rule.layer), other=layer(self.rule.other)
        )

    def initAlgorithm(self, configuration: Parameters | None = None):
        self.addParameter(
            QgsProcessingParameterFeatureSource(
                INPUT, tr('Input layer'), layer_types(self.rule.layer)
            )
        )
        if self.rule.other is not None:
            self.addParameter(
                QgsProcessingParameterFeatureSource(
                    OTHER, tr('Other layer'), layer_types(self.rule.other)
                )
            )
        self.addParameter(
            QgsProcessingParameterDistance(
                TOLERANCE,
                tr('Tolerance (empty for the default of the CRS)'),
                parentParameterName=INPUT,
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
        """Reads the layers and the settings, which belong to the main
        thread, so that the run only has to use them."""

        def prepare() -> None:
            first = self.read(parameters, INPUT, context, self.rule.layer)
            other = None
            if self.rule.other is not None:
                other = self.read(parameters, OTHER, context, self.rule.other)
            sinks = self.sink_files(parameters, context)
            refuse_overwrite(sinks, (first, other))
            release_files(sinks)
            self.plan = Plan(
                engine.tool(),
                first,
                other,
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
            outcome = run_jobs(
                [Job(self.rule, plan.first, plan.other)],
                plan.tool,
                plan.tolerance,
                path,
                Path(tmp),
                context,
                feedback,
            )
            with memory_copy(path) as source:
                results = self.write_sinks(
                    parameters,
                    context,
                    outcome.crs,
                    source,
                    outcome.ids,
                    outcome.invalid,
                    self.rule.display_name,
                )
        count = sum(outcome.counts) + len(outcome.invalid)
        feedback.pushInfo(tr('{count} topology errors').format(count=count))
        return {**results, COUNT: count}

    def read(
        self,
        parameters: Parameters,
        name: str,
        context: QgsProcessingContext,
        kind: str,
    ) -> Input:
        source = self.parameterAsSource(parameters, name, context)
        if source is None:
            raise QgsProcessingException(
                self.invalidSourceError(parameters, name)
            )
        layer = self.parameterAsVectorLayer(parameters, name, context)
        value = parameters.get(name)
        whole = not (
            isinstance(value, QgsProcessingFeatureSourceDefinition)
            and (
                value.selectedFeaturesOnly
                or value.featureLimit > 0
                or getattr(value, 'filterExpression', '')
            )
        )
        item = Input(
            layer.name() if layer is not None else name, source, layer, whole
        )
        require_kind(item, kind)
        return item
