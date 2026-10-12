"""Native planar topology checks for Python GIS workflows."""

from ._api import Topology, explode_lines, merge_lines
from ._enums import ErrorType, GeometryKind, Rule, RuleVariant
from ._errors import PrecisionError
from ._models import FeatureRef, Layer, LayerId, Report, RuleReport, RuleSpec, Violation
from ._stream import ReportSummary, RuleSummary, ValidationStream

__all__ = [
    "ErrorType",
    "PrecisionError",
    "FeatureRef",
    "GeometryKind",
    "Layer",
    "LayerId",
    "Report",
    "ReportSummary",
    "RuleSummary",
    "ValidationStream",
    "Rule",
    "RuleReport",
    "RuleSpec",
    "RuleVariant",
    "Topology",
    "Violation",
    "explode_lines",
    "merge_lines",
]
