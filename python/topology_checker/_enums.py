"""Typed public rule names and native registry metadata."""

from __future__ import annotations

import json
from dataclasses import dataclass
from enum import Enum
from typing import Any

from . import _native


class GeometryKind(Enum):
    """Geometry families accepted by a rule."""

    POINT = "point"
    LINE = "line"
    POLYGON = "polygon"
    COLLECTION = "collection"
    ANY = "any"


class ErrorType(Enum):
    """Deterministically ordered topology violation categories."""

    OVERLAP = "overlap"
    OUTSIDE = "outside"
    DANGLE = "dangle"
    PSEUDO_NODE = "pseudo-node"
    CROSSING = "crossing"
    TOUCH_INTERIOR = "touch-interior"
    SELF_OVERLAP = "self-overlap"
    SELF_CROSSING = "self-crossing"
    GAP = "gap"
    OUTER_BOUNDARY = "outer-boundary"
    MULTIPART = "multipart"
    NOT_COVERED = "not-covered"
    MISSING_POINT = "missing-point"
    POINT_COUNT = "point-count"
    CONTAINED = "contained"
    INTERSECTION = "intersection"


@dataclass(frozen=True)
class RuleVariant:
    """Canonical Rust rule semantics and ordered input families."""

    value: str
    inputs: tuple[GeometryKind, GeometryKind | None]
    description: str
    arcgis: str | None
    qgis: str | None


class Rule(Enum):
    """Geometry-independent rules in first-occurrence Rust registry order."""

    MUST_NOT_OVERLAP = "must-not-overlap"
    MUST_BE_INSIDE = "must-be-inside"
    MUST_NOT_HAVE_DANGLES = "must-not-have-dangles"
    MUST_NOT_HAVE_PSEUDO_NODES = "must-not-have-pseudo-nodes"
    MUST_NOT_SELF_OVERLAP = "must-not-self-overlap"
    MUST_NOT_INTERSECT = "must-not-intersect"
    MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR = "must-not-intersect-or-touch-interior"
    MUST_NOT_SELF_INTERSECT = "must-not-self-intersect"
    MUST_NOT_HAVE_GAPS = "must-not-have-gaps"
    MUST_NOT_BE_MULTIPART = "must-not-be-multipart"
    MUST_BE_COVERED_BY = "must-be-covered-by"
    MUST_BE_COVERED_BY_ENDPOINTS = "must-be-covered-by-endpoints"
    MUST_BE_COVERED_BY_BOUNDARIES = "must-be-covered-by-boundaries"
    ENDPOINTS_MUST_BE_COVERED_BY = "endpoints-must-be-covered-by"
    BOUNDARIES_MUST_BE_COVERED_BY = "boundaries-must-be-covered-by"
    MUST_CONTAIN = "must-contain"
    MUST_CONTAIN_ONE = "must-contain-one"
    MUST_NOT_CROSS_BOUNDARIES = "must-not-cross-boundaries"
    MUST_NOT_BE_CONTAINED_BY = "must-not-be-contained-by"
    MUST_NOT_BE_INSIDE = "must-not-be-inside"
    MUST_BE_PROPERLY_INSIDE = "must-be-properly-inside"
    MUST_NOT_OVERLAP_BOUNDARIES = "must-not-overlap-boundaries"
    ENDPOINTS_MUST_BE_COVERED_BY_BOUNDARIES = "endpoints-must-be-covered-by-boundaries"
    MUST_NOT_CONTAIN = "must-not-contain"

    @property
    def description(self) -> str:
        """Return a geometry-independent description of this constraint."""
        return self.value.replace("-", " ").capitalize() + "."

    @property
    def variants(self) -> tuple[RuleVariant, ...]:
        """Return concrete semantics in Rust registry order."""
        return _VARIANTS[self]


def resolve(rule: Rule, kinds: tuple[GeometryKind | None, ...]) -> RuleVariant:
    """Select one ordered signature, leaving unknown families unconstrained.

    Args:
        rule: Generic public constraint.
        kinds: Primary and optional secondary geometry families.

    Returns:
        The uniquely matching native rule variant.

    Raises:
        ValueError: Input families or arity are unsupported or ambiguous.
    """
    candidates: list[RuleVariant] = []
    signatures: list[str] = []
    for variant in rule.variants:
        inputs = tuple(kind for kind in variant.inputs if kind is not None)
        signatures.append(" → ".join(kind.value for kind in inputs))
        if len(inputs) == len(kinds) and all(
            actual is None or expected is GeometryKind.ANY or actual is expected
            for actual, expected in zip(kinds, inputs, strict=True)
        ):
            candidates.append(variant)
    if len(candidates) == 1:
        return candidates[0]
    reason = "ambiguous" if candidates else "unsupported"
    provided = " → ".join(kind.value if kind else "unknown" for kind in kinds)
    raise ValueError(
        f"{reason} inputs for {rule.value}: {provided}; "
        f"supported signatures: {', '.join(signatures)}. "
        "Declare an empty layer's kind with add_layer(data, kind=GeometryKind.POINT) "
        "(or GeometryKind.LINE or GeometryKind.POLYGON)."
    )


_GROUPS: dict[Rule, tuple[str, ...]] = {
    Rule.MUST_NOT_OVERLAP: (
        "point/must-not-overlap",
        "point/must-not-overlap-with",
        "line/must-not-overlap",
        "line/must-not-overlap-with",
        "polygon/must-not-overlap",
        "polygon/must-not-overlap-with",
        "line/must-not-overlap-with-polygons",
    ),
    Rule.MUST_BE_INSIDE: (
        "point/must-be-inside",
        "line/must-be-inside",
    ),
    Rule.MUST_NOT_HAVE_DANGLES: ("line/must-not-have-dangles",),
    Rule.MUST_NOT_HAVE_PSEUDO_NODES: ("line/must-not-have-pseudo-nodes",),
    Rule.MUST_NOT_SELF_OVERLAP: ("line/must-not-self-overlap",),
    Rule.MUST_NOT_INTERSECT: (
        "line/must-not-intersect",
        "line/must-not-intersect-with",
        "point/must-not-intersect-lines",
        "point/must-not-intersect-polygons",
        "line/must-not-intersect-polygons",
    ),
    Rule.MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR: (
        "line/must-not-intersect-or-touch-interior",
        "line/must-not-intersect-or-touch-interior-with",
    ),
    Rule.MUST_NOT_SELF_INTERSECT: ("line/must-not-self-intersect",),
    Rule.MUST_NOT_HAVE_GAPS: ("polygon/must-not-have-gaps",),
    Rule.MUST_NOT_BE_MULTIPART: ("geometry/must-not-be-multipart",),
    Rule.MUST_BE_COVERED_BY: (
        "point/must-be-covered-by-lines",
        "point/must-be-covered-by-polygons",
    ),
    Rule.MUST_BE_COVERED_BY_ENDPOINTS: ("point/must-be-covered-by-line-endpoints",),
    Rule.MUST_BE_COVERED_BY_BOUNDARIES: (
        "point/must-be-covered-by-polygon-boundaries",
        "line/must-be-covered-by-polygon-boundaries",
    ),
    Rule.ENDPOINTS_MUST_BE_COVERED_BY: ("line/endpoints-must-be-covered-by-points",),
    Rule.BOUNDARIES_MUST_BE_COVERED_BY: (
        "polygon/boundaries-must-be-covered-by-lines",
    ),
    Rule.MUST_CONTAIN: ("polygon/must-contain-point",),
    Rule.MUST_CONTAIN_ONE: ("polygon/must-contain-one-point",),
    Rule.MUST_NOT_CROSS_BOUNDARIES: ("line/must-not-cross-polygon-boundaries",),
    Rule.MUST_NOT_BE_CONTAINED_BY: ("geometry/must-not-be-contained-by",),
    Rule.MUST_NOT_BE_INSIDE: ("point/must-not-be-inside-polygons",),
    Rule.MUST_BE_PROPERLY_INSIDE: ("line/must-be-properly-inside-polygons",),
    Rule.MUST_NOT_OVERLAP_BOUNDARIES: ("line/must-not-overlap-polygon-boundaries",),
    Rule.ENDPOINTS_MUST_BE_COVERED_BY_BOUNDARIES: (
        "line/endpoints-must-be-covered-by-polygon-boundaries",
    ),
    Rule.MUST_NOT_CONTAIN: ("polygon/must-not-contain-points",),
}
_METADATA: list[dict[str, Any]] = json.loads(_native.registry())
_VARIANTS: dict[Rule, tuple[RuleVariant, ...]] = {
    rule: tuple(
        RuleVariant(
            item["value"],
            (
                GeometryKind(item["inputs"][0]),
                None if item["inputs"][1] is None else GeometryKind(item["inputs"][1]),
            ),
            item["description"],
            item["arcgis"],
            item["qgis"],
        )
        for item in _METADATA
        if item["value"] in values
    )
    for rule, values in _GROUPS.items()
}
