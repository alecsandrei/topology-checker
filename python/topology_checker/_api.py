"""Public Python topology facade over the persistent Rust engine."""

from __future__ import annotations

import json
import math
import threading
from collections.abc import Generator, Iterable
from contextlib import contextmanager
from types import MappingProxyType
from typing import Any, cast

import shapely
from pyproj import CRS
from shapely.geometry import LineString

from . import _native
from ._enums import ErrorType, GeometryKind, Rule, resolve
from ._input import Prepared, prepare
from ._models import (
    FeatureRef,
    Layer,
    LayerId,
    Report,
    RuleReport,
    RuleSpec,
    Violation,
    new_layer_id,
)
from ._wire import NativeReference, NativeReport


def _tolerance(value: object) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError("tolerance must be a number")
    value = float(value)
    if not math.isfinite(value) or value < 0:
        raise ValueError("tolerance must be finite and nonnegative")
    return value


class Topology:
    """Own planar layers, ordered rules, and reusable native topology caches.

    Args:
        tolerance: Nonnegative finite distance in input coordinate units.

    Raises:
        ValueError: Tolerance is negative or nonfinite.
    """

    def __init__(self, tolerance: float = 0.0) -> None:
        self._tolerance = _tolerance(tolerance)
        self._native = _native.NativeTopology(self._tolerance)
        self._owner = object()
        self._layers: list[Layer] = []
        self._specs: list[RuleSpec] = []
        self._lock = threading.RLock()
        self._registration_active = False

    @contextmanager
    def _registration(self) -> Generator[None, None, None]:
        """Serialize mutations and reject registration from input callbacks."""
        with self._lock:
            if self._registration_active:
                raise RuntimeError(
                    "registration cannot be reentered while preparing GIS input"
                )
            self._registration_active = True
            try:
                yield
            finally:
                self._registration_active = False

    @property
    def tolerance(self) -> float:
        """Return the planar tolerance in caller-supplied coordinate units."""
        return self._tolerance

    @property
    def layers(self) -> tuple[Layer, ...]:
        """Return immutable layer snapshots in registration order."""
        with self._lock:
            return tuple(self._layers)

    def _id(self, layer: object) -> int:
        if not isinstance(layer, LayerId):
            raise TypeError("layer must be a LayerId registered by this topology")
        if not layer.belongs_to(self._owner) or not 0 <= layer.index < len(
            self._layers
        ):
            raise ValueError("layer ID does not belong to this topology")
        return layer.index

    def layer(self, id: LayerId) -> Layer:
        """Return a registered layer, rejecting IDs from other topologies."""
        with self._lock:
            return self._layers[self._id(id)]

    def add_layer(
        self,
        data: Any,
        *,
        name: str | None = None,
        fids: Iterable[int] | None = None,
        crs: Any = None,
        kind: GeometryKind | None = None,
    ) -> LayerId:
        """Snapshot and register GIS data as a planar layer.

        Args:
            data: Geometry iterable, GeoSeries, GeoDataFrame, GeoJSON mapping,
                or object implementing __geo_interface__.
            name: Optional display label. Defaults to layer_N using registration order.
            fids: Unique signed 64-bit IDs for all original input positions.
            crs: Optional CRS, which must agree with input and known layer metadata.
            kind: Optional POINT, LINE, or POLYGON declaration for ambiguous data.

        Returns:
            An identity scoped to this topology.

        Raises:
            TypeError: Input data, the name, or the declaration has an unsupported type.
            ValueError: Coordinates, FIDs, geometry, or CRS metadata are invalid.
            RuntimeError: An input callback tries to register on this topology.
        """
        if name is not None and not isinstance(cast(object, name), str):
            raise TypeError("name must be a string or None")
        with self._registration():
            label = f"layer_{len(self._layers)}" if name is None else name
            prepared = prepare(data, label, fids, crs, kind)
            self._check_crs([prepared])
            index = self._native.add_layer(label, list(prepared.features))
            snapshot = self._snapshot(prepared, label, index)
            self._layers.append(snapshot)
            return snapshot.id

    def _check_crs(self, pending: Iterable[Prepared]) -> None:
        """Check registered and pending CRS metadata before native mutation."""
        known = [layer.crs for layer in self._layers if layer.crs is not None]
        known.extend(item.crs for item in pending if item.crs is not None)
        if known and any(crs != known[0] for crs in known[1:]):
            raise ValueError("incompatible known CRSs; reproject inputs first")

    def _snapshot(self, prepared: Prepared, name: str, index: int) -> Layer:
        """Create an immutable metadata view of one prepared native layer."""
        return Layer(
            new_layer_id(self._owner, index),
            name,
            tuple(fid for fid, _ in prepared.features),
            prepared.geometries,
            prepared.crs,
            MappingProxyType(prepared.parts),
            prepared.warnings,
            prepared.kind,
        )

    def add_rule(self, rule: Rule, data: Any, *, other: Any = None) -> RuleSpec:
        """Snapshot raw GIS inputs or reuse IDs, then atomically register a rule.

        Each raw input creates a fresh snapshot. Pass a LayerId to reuse a layer.

        Args:
            rule: Rule member. Deserialize strings explicitly with Rule(value).
            data: Primary GIS input or a LayerId from this topology.
            other: Optional secondary GIS input or distinct registered LayerId.

        Returns:
            The immutable specification, including reusable IDs and native semantics.

        Raises:
            TypeError: The rule or input has an unsupported type.
            ValueError: IDs, families, arity, coordinates, FIDs, or CRS are invalid.
                Failed calls leave registrations and visible warnings unchanged.
            RuntimeError: An input callback tries to register on this topology.
        """
        if not isinstance(cast(object, rule), Rule):
            raise TypeError("rule must be a Rule enum member; use Rule(value)")
        with self._registration():
            inputs: list[tuple[int | None, str, list[tuple[int, bytes]]]] = []
            pending: list[Layer] = []
            prepared_inputs: list[Prepared] = []
            ids: list[LayerId] = []
            kinds: list[GeometryKind | None] = []
            for value in [data] if other is None else [data, other]:
                if isinstance(value, LayerId):
                    index = self._id(value)
                    inputs.append((index, "", []))
                    ids.append(value)
                    kinds.append(self._layers[index].kind)
                else:
                    index = len(self._layers) + len(pending)
                    name = f"layer_{index}"
                    prepared = prepare(value, name)
                    snapshot = self._snapshot(prepared, name, index)
                    pending.append(snapshot)
                    prepared_inputs.append(prepared)
                    inputs.append((None, name, list(prepared.features)))
                    ids.append(snapshot.id)
                    kinds.append(prepared.kind)
            self._check_crs(prepared_inputs)
            variant = resolve(rule, tuple(kinds))
            spec = RuleSpec(rule, ids[0], None if other is None else ids[1], variant)
            self._native.register_rule(variant.value, inputs)
            self._layers.extend(pending)
            self._specs.append(spec)
            return spec

    def _crs(self) -> tuple[CRS | None, tuple[str, ...]]:
        known = [layer.crs for layer in self._layers if layer.crs is not None]
        if known and len(known) != len(self._layers):
            return None, ("mixed known and unknown CRS metadata; export CRS is unset",)
        return known[0] if known else None, ()

    def _warnings(self, native: Iterable[str]) -> tuple[str, ...]:
        return (
            tuple(w for layer in self._layers for w in layer.warnings)
            + tuple(native)
            + self._crs()[1]
        )

    def warnings(self) -> tuple[str, ...]:
        """Return input, degenerate-part, and combined CRS warnings."""
        with self._lock:
            return self._warnings(self._native.warnings())

    def _reference(self, value: NativeReference | None) -> FeatureRef | None:
        if value is None:
            return None
        layer = self._layers[value["layer"]]
        return layer.reference(value["fid"], value["part"])

    def validate(self) -> Report:
        """Run Rust validation with the GIL released and return a typed snapshot.

        Returns:
            A report whose violations are data, including when rules fail.

        Raises:
            RuntimeError: An unexpected native failure prevents validation.
        """
        with self._lock:
            result = cast(NativeReport, json.loads(self._native.validate()))
            rules: list[RuleReport] = []
            for spec, item in zip(self._specs, result["rules"], strict=True):
                primary = self._layers[item["layer"]]
                other = None if item["other"] is None else self._layers[item["other"]]
                errors = tuple(
                    Violation(
                        spec.rule,
                        ErrorType(error["error_type"]),
                        shapely.from_wkb(bytes(error["wkb"])),
                        self._reference(error["origin"]),
                        self._reference(error["destination"]),
                        spec.resolved_rule,
                    )
                    for error in item["errors"]
                )
                counts = MappingProxyType(
                    {
                        category: item["counts"][category.value]
                        for category in ErrorType
                        if category.value in item["counts"]
                    }
                )
                rules.append(
                    RuleReport(
                        spec.rule,
                        primary.id,
                        None if other is None else other.id,
                        primary.name,
                        None if other is None else other.name,
                        counts,
                        errors,
                        spec,
                    )
                )
            return Report(
                result["tolerance"],
                self._warnings(result["warnings"]),
                tuple(rules),
                self._crs()[0],
            )


def merge_lines(data: Any, tolerance: float = 0.0) -> tuple[LineString, ...]:
    """Merge line parts through degree-two endpoint joins in native Rust.

    Args:
        data: Any supported layer input containing only line geometries.
        tolerance: Nonnegative finite distance in planar coordinate units.

    Returns:
        Shapely LineStrings in deterministic contributing-part order.
    """
    value = _tolerance(tolerance)
    prepared = prepare(data, "merge-lines")
    return tuple(
        cast(LineString, shapely.from_wkb(bytes(wkb)))
        for wkb in _native.lines([wkb for _, wkb in prepared.features], value)
    )


def explode_lines(data: Any) -> tuple[LineString, ...]:
    """Return native line segments preserving input part order and direction.

    Args:
        data: Any supported layer input containing only line geometries.

    Returns:
        Shapely LineStrings, with degenerate parts skipped.
    """
    prepared = prepare(data, "explode-lines")
    return tuple(
        cast(LineString, shapely.from_wkb(bytes(wkb)))
        for wkb in _native.lines([wkb for _, wkb in prepared.features])
    )
