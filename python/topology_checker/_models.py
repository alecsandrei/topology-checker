"""Immutable public layer and validation report views."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from functools import cached_property
from typing import TYPE_CHECKING, Any, cast

from . import _optional
from ._enums import ErrorType, GeometryKind, Rule, RuleVariant

if TYPE_CHECKING:
    from geopandas import GeoDataFrame
    from pyproj import CRS
    from shapely.geometry.base import BaseGeometry


@dataclass(frozen=True, init=False)
class LayerId:
    """Opaque identity scoped to the Topology that registered a layer."""

    _owner: object = field(repr=False)
    _index: int

    def __new__(cls) -> LayerId:
        raise TypeError(
            "LayerId objects are returned by Topology.add_layer or Topology.add_rule"
        )

    @property
    def index(self) -> int:
        """Return the zero-based registration index."""
        return self._index

    def __repr__(self) -> str:
        return f"LayerId({self.index})"

    def belongs_to(self, owner: object) -> bool:
        """Check the internal topology identity token."""
        return self._owner is owner


@dataclass(frozen=True)
class Layer:
    """Immutable XY snapshot of retained features and layer metadata.

    The kind is a concrete retained family, COLLECTION for mixed families, or
    None when unknown. Empty inputs can retain a typed hint or an explicit kind.
    `wkb` holds one 2D little-endian ISO WKB snapshot per retained FID.
    """

    id: LayerId
    name: str
    fids: tuple[int, ...]
    wkb: tuple[bytes, ...] = field(repr=False)
    crs: CRS | None
    _parts: Mapping[int, tuple[int, ...]] = field(repr=False, compare=False)
    warnings: tuple[str, ...] = field(repr=False)
    kind: GeometryKind | None

    def __len__(self) -> int:
        return len(self.fids)

    @cached_property
    def geometries(self) -> tuple[BaseGeometry, ...]:
        """Decode the snapshots as shapely geometries.

        Raises:
            ImportError: Shapely is missing; install topology-checker[io].
        """
        shapely = _optional.shapely()
        decoded = shapely.from_wkb(list(self.wkb))  # pyright: ignore[reportUnknownMemberType]
        return tuple(cast("list[BaseGeometry]", decoded.tolist()))

    def reference(self, fid: int, part: int | None) -> FeatureRef:
        """Map a native part index to original feature provenance."""
        if part is not None and (positions := self._parts.get(fid)) is not None:
            part = positions[part]
        return FeatureRef(self.id, fid, part)


@dataclass(frozen=True)
class FeatureRef:
    """Original feature and depth-first part position in a registered layer."""

    layer: LayerId
    fid: int
    part: int | None

    def to_dict(self) -> dict[str, int | None]:
        """Serialize a reference using its registration index."""
        return {"layer": self.layer.index, "fid": self.fid, "part": self.part}


def new_layer_id(owner: object, index: int) -> LayerId:
    """Create an identity internally; public construction is unsupported."""
    instance = object.__new__(LayerId)
    object.__setattr__(instance, "_owner", owner)
    object.__setattr__(instance, "_index", index)
    return instance


@dataclass(frozen=True)
class RuleSpec:
    """A registered generic constraint and its selected native semantics."""

    rule: Rule
    layer: LayerId
    other: LayerId | None
    resolved_rule: RuleVariant


@dataclass(frozen=True)
class Violation:
    """A topology violation with WKB geometry and original provenance."""

    rule: Rule
    error_type: ErrorType
    _wkb: bytes = field(repr=False)
    origin: FeatureRef | None
    destination: FeatureRef | None
    resolved_rule: RuleVariant

    @property
    def wkb(self) -> bytes:
        """Return the geometry as 2D little-endian ISO WKB."""
        return self._wkb

    @cached_property
    def geometry(self) -> BaseGeometry:
        """Decode the geometry as a shapely geometry.

        Raises:
            ImportError: Shapely is missing; install topology-checker[io].
        """
        return _optional.shapely().from_wkb(self._wkb)

    def to_dict(self) -> dict[str, Any]:
        """Serialize enums as values and geometry as a GeoJSON mapping.

        Raises:
            ImportError: Shapely is missing; install topology-checker[io].
        """
        shapely = _optional.shapely()
        return {
            "rule": self.rule.value,
            "resolved_rule": self.resolved_rule.value,
            "error_type": self.error_type.value,
            "geometry": shapely.geometry.mapping(self.geometry),
            "origin": None if self.origin is None else self.origin.to_dict(),
            "destination": (
                None if self.destination is None else self.destination.to_dict()
            ),
        }


@dataclass(frozen=True)
class RuleReport:
    """One registered rule's ordered violations and category counts."""

    rule: Rule
    layer: LayerId
    other: LayerId | None
    layer_name: str
    other_name: str | None
    counts: Mapping[ErrorType, int]
    violations: tuple[Violation, ...]
    spec: RuleSpec

    @property
    def resolved_rule(self) -> RuleVariant:
        """Return the concrete rule selected during registration."""
        return self.spec.resolved_rule

    @property
    def errors(self) -> tuple[Violation, ...]:
        """Return the violations, matching Rust's report terminology."""
        return self.violations

    @property
    def is_valid(self) -> bool:
        """Return whether this rule reported no violations."""
        return not self.violations

    @property
    def error_count(self) -> int:
        """Return the number of aggregated violations."""
        return len(self.violations)

    def to_dict(self) -> dict[str, Any]:
        """Serialize this ordered rule result.

        Raises:
            ImportError: Violations exist and shapely is missing; install
                topology-checker[io].
        """
        return {
            "rule": self.rule.value,
            "resolved_rule": self.resolved_rule.value,
            "layer": self.layer.index,
            "other": None if self.other is None else self.other.index,
            "layer_name": self.layer_name,
            "other_name": self.other_name,
            "counts": {key.value: value for key, value in self.counts.items()},
            "errors": [violation.to_dict() for violation in self.violations],
        }


@dataclass(frozen=True)
class Report:
    """A validation snapshot, independent of subsequent registrations."""

    tolerance: float
    warnings: tuple[str, ...]
    rules: tuple[RuleReport, ...]
    crs: CRS | None

    @property
    def violations(self) -> tuple[Violation, ...]:
        """Flatten violations in rule order and native deterministic order."""
        return tuple(error for rule in self.rules for error in rule.violations)

    @property
    def errors(self) -> tuple[Violation, ...]:
        """Return flattened violations, matching Rust's report terminology."""
        return self.violations

    @property
    def is_valid(self) -> bool:
        """Return whether every registered rule passed."""
        return all(rule.is_valid for rule in self.rules)

    @property
    def error_count(self) -> int:
        """Return the total number of aggregated violations."""
        return sum(rule.error_count for rule in self.rules)

    def to_dict(self) -> dict[str, Any]:
        """Return JSON-compatible metadata, GeoJSON geometries, and enum values.

        Raises:
            ImportError: Violations exist and shapely is missing; install
                topology-checker[io].
        """
        return {
            "tolerance": self.tolerance,
            "warnings": list(self.warnings),
            "crs": None if self.crs is None else self.crs.to_string(),
            "is_valid": self.is_valid,
            "error_count": self.error_count,
            "rules": [rule.to_dict() for rule in self.rules],
        }

    def to_geodataframe(self) -> GeoDataFrame:
        """Export one row per violation, with scalar provenance columns.

        Returns:
            A GeoDataFrame retaining a common known CRS, including for no errors.

        Raises:
            ImportError: GeoPandas is missing; install topology-checker[io].
        """
        try:
            import geopandas as gpd
            import pandas as pd
        except ImportError as exc:
            raise ImportError(
                "GeoDataFrame export requires topology-checker[io]"
            ) from exc
        columns = [
            "rule",
            "resolved_rule",
            "error_type",
            "layer",
            "other",
            "layer_name",
            "other_name",
            "origin_layer",
            "origin_fid",
            "origin_part",
            "destination_layer",
            "destination_fid",
            "destination_part",
            "geometry",
        ]
        rows: list[dict[str, Any]] = []
        for rule in self.rules:
            for violation in rule.violations:
                row: dict[str, Any] = {
                    "rule": violation.rule.value,
                    "resolved_rule": violation.resolved_rule.value,
                    "error_type": violation.error_type.value,
                    "layer": rule.layer.index,
                    "other": None if rule.other is None else rule.other.index,
                    "layer_name": rule.layer_name,
                    "other_name": rule.other_name,
                    "geometry": violation.geometry,
                }
                for prefix, ref in (
                    ("origin", violation.origin),
                    ("destination", violation.destination),
                ):
                    row[f"{prefix}_layer"] = None if ref is None else ref.layer.index
                    row[f"{prefix}_fid"] = None if ref is None else ref.fid
                    row[f"{prefix}_part"] = None if ref is None else ref.part
                rows.append(row)
        frame = pd.DataFrame(rows, columns=columns)
        # Construct nullable integer arrays directly to avoid a float conversion
        # losing large signed FIDs when another row has absent provenance.
        integer_columns = ["layer", "other", *columns[7:13]]
        for column in integer_columns:
            frame[column] = pd.Series([row[column] for row in rows], dtype="Int64")
        return gpd.GeoDataFrame(frame, geometry="geometry", crs=self.crs)
