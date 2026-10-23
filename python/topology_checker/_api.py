"""Public Python topology facade over the persistent Rust engine."""

from __future__ import annotations

import math
import re
import threading
from collections.abc import Generator, Iterable
from contextlib import contextmanager
from types import MappingProxyType
from typing import TYPE_CHECKING, Any, cast, overload

from . import _native, _optional
from ._enums import ErrorType, GeometryKind, Rule, resolve
from ._input import Prepared, prepare
from ._models import (
    Layer,
    LayerId,
    Report,
    RuleReport,
    RuleSpec,
    Violation,
    new_layer_id,
)
from ._stream import ValidationStream, decode, warning_messages

if TYPE_CHECKING:
    from pyproj import CRS
    from shapely.geometry import LineString

WKBItem = bytes | bytearray | memoryview


def _tolerance(value: object) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError("tolerance must be a number")
    value = float(value)
    if not math.isfinite(value) or value < 0:
        raise ValueError("tolerance must be finite and nonnegative")
    return value


def _original_parts(message: str, inputs: list[tuple[str, Prepared]]) -> str:
    """Retain original empty-leaf positions in invalid polygon diagnostics."""
    match = re.search(r"layer (.*), FID (-?\d+), part (\d+), ring", message)
    if match is None:
        return message
    name, fid, part = match.group(1), int(match.group(2)), int(match.group(3))
    for label, prepared in inputs:
        positions = prepared.parts.get(fid, ())
        if label == name and part < len(positions):
            return (
                message[: match.start(3)]
                + str(positions[part])
                + message[match.end(3) :]
            )
    return message


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

        WKB items (bytes, bytearray or memoryview, ISO or extended WKB) need no
        optional dependency. None items are missing features. WKB carries no CRS.

        Args:
            data: Sequence of WKB items or shapely geometries, GeoSeries,
                GeoDataFrame, GeoJSON mapping, or object implementing
                __geo_interface__.
            name: Optional display label. Defaults to layer_N using registration order.
            fids: Unique signed 64-bit IDs for all original input positions.
            crs: Optional CRS, which must agree with input and known layer metadata.
            kind: Optional POINT, LINE, or POLYGON declaration for ambiguous data.

        Returns:
            An identity scoped to this topology.

        Raises:
            TypeError: Input data, the name, or the declaration has an unsupported type.
            ValueError: Coordinates, FIDs, geometry, or CRS metadata are invalid,
                or a WKB item is unreadable; the message names the layer and FID.
            ImportError: Shapely, GeoJSON or CRS input needs topology-checker[io].
            RuntimeError: An input callback tries to register on this topology.
        """
        if name is not None and not isinstance(cast(object, name), str):
            raise TypeError("name must be a string or None")
        with self._registration():
            label = f"layer_{len(self._layers)}" if name is None else name
            prepared = prepare(data, label, fids, crs, kind)
            self._check_crs([prepared])
            try:
                index = self._native.add_layer(label, list(prepared.features))
            except ValueError as error:
                raise ValueError(
                    _original_parts(str(error), [(label, prepared)])
                ) from error
            snapshot = self._snapshot(prepared, label, index)
            self._layers.append(snapshot)
            return snapshot.id

    def _check_crs(self, pending: Iterable[Prepared]) -> None:
        """Check registered and pending CRS metadata before native mutation."""
        known = [layer.crs for layer in self._layers if layer.crs is not None]
        known.extend(item.crs for item in pending if item.crs is not None)
        if known and any(
            not crs.equals(known[0], ignore_axis_order=True) for crs in known[1:]
        ):
            raise ValueError("incompatible known CRSs; reproject inputs first")

    def _snapshot(self, prepared: Prepared, name: str, index: int) -> Layer:
        """Create an immutable metadata view of one prepared native layer."""
        return Layer(
            new_layer_id(self._owner, index),
            name,
            tuple(fid for fid, _ in prepared.features),
            prepared.wkb,
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
            data: Primary input accepted by add_layer, or a LayerId from this
                topology.
            other: Optional secondary input or distinct registered LayerId.

        Returns:
            The immutable specification, including reusable IDs and native semantics.

        Raises:
            TypeError: The rule or input has an unsupported type.
            ValueError: IDs, families, arity, coordinates, FIDs, WKB, or CRS are
                invalid. Failed calls leave registrations and visible warnings
                unchanged.
            ImportError: Shapely, GeoJSON or CRS input needs topology-checker[io].
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
            try:
                self._native.register_rule(variant.value, inputs)
            except ValueError as error:
                raise ValueError(
                    _original_parts(
                        str(error),
                        [
                            (layer.name, item)
                            for layer, item in zip(
                                pending, prepared_inputs, strict=True
                            )
                        ],
                    )
                ) from error
            self._layers.extend(pending)
            self._specs.append(spec)
            return spec

    def _crs(self) -> tuple[CRS | None, tuple[str, ...]]:
        known = [layer.crs for layer in self._layers if layer.crs is not None]
        if known and len(known) != len(self._layers):
            return None, ("mixed known and unknown CRS metadata; export CRS is unset",)
        return known[0] if known else None, ()

    def _warnings(self, native: Iterable[_native.NativeWarning]) -> tuple[str, ...]:
        return (
            tuple(w for layer in self._layers for w in layer.warnings)
            + warning_messages(list(native), tuple(self._layers))
            + self._crs()[1]
        )

    def warnings(self) -> tuple[str, ...]:
        """Return input, degenerate-part, and combined CRS warnings."""
        with self._lock:
            return self._warnings(self._native.warnings())

    def iter_validate(self, batch_size: int = 1024) -> ValidationStream:
        """Stream a snapshot in bounded binary batches, closing it on early exit.

        Args:
            batch_size: Positive maximum number of buffered violations.

        Returns:
            A closeable iterator, with final summary after exhaustion.
        """
        if isinstance(batch_size, bool) or not isinstance(
            cast(object, batch_size), int
        ):
            raise TypeError("batch_size must be an integer")
        if batch_size <= 0:
            raise ValueError("batch_size must be positive")
        with self._lock:
            return ValidationStream(
                self._native.iter_validate(batch_size),
                tuple(self._layers),
                tuple(self._specs),
                self._warnings(()),
                self._crs()[0],
            )

    def validate(self) -> Report:
        """Run native validation and return an eager immutable report."""
        with self._lock:
            result = self._native.validate()
            errors: list[list[Violation]] = [[] for _ in self._specs]
            layers, specs = tuple(self._layers), tuple(self._specs)
            for error in result.errors:
                errors[error.rule_index].append(decode(error, layers, specs))
            rules = tuple(
                RuleReport(
                    spec.rule,
                    spec.layer,
                    spec.other,
                    self._layers[item.layer].name,
                    None if item.other is None else self._layers[item.other].name,
                    MappingProxyType(
                        {ErrorType(key): count for key, count in item.counts.items()}
                    ),
                    tuple(errors[i]),
                    spec,
                )
                for i, (spec, item) in enumerate(
                    zip(self._specs, result.summary.rules, strict=True)
                )
            )
            return Report(
                result.summary.tolerance,
                self._warnings(result.summary.warnings),
                rules,
                self._crs()[0],
            )


def _lines(prepared: Prepared, output: list[bytes]) -> tuple[Any, ...]:
    """Return WKB for WKB input, otherwise shapely LineStrings."""
    if prepared.wkb_only:
        return tuple(output)
    shapely = _optional.shapely()
    decoded = shapely.from_wkb(output)  # pyright: ignore[reportUnknownMemberType]
    return tuple(cast("list[LineString]", decoded.tolist()))


# WKB-only input returns WKB; the broader overload covers every other input.
@overload
def merge_lines(  # pyright: ignore[reportOverlappingOverload]
    data: Iterable[WKBItem | None], tolerance: float = 0.0
) -> tuple[bytes, ...]: ...
@overload
def merge_lines(data: Any, tolerance: float = 0.0) -> tuple[LineString, ...]: ...
def merge_lines(
    data: Any, tolerance: float = 0.0
) -> tuple[bytes, ...] | tuple[LineString, ...]:
    """Merge line parts through degree-two endpoint joins in native Rust.

    Args:
        data: Any supported layer input containing only line geometries.
        tolerance: Nonnegative finite distance in planar coordinate units.

    Returns:
        Lines in deterministic contributing-part order: 2D ISO WKB for WKB
        input, otherwise shapely LineStrings.
    """
    value = _tolerance(tolerance)
    prepared = prepare(data, "merge-lines")
    return _lines(prepared, _native.lines([wkb for _, wkb in prepared.features], value))


@overload
def explode_lines(  # pyright: ignore[reportOverlappingOverload]
    data: Iterable[WKBItem | None],
) -> tuple[bytes, ...]: ...
@overload
def explode_lines(data: Any) -> tuple[LineString, ...]: ...
def explode_lines(data: Any) -> tuple[bytes, ...] | tuple[LineString, ...]:
    """Return native line segments preserving input part order and direction.

    Args:
        data: Any supported layer input containing only line geometries.

    Returns:
        Segments, with degenerate parts skipped: 2D ISO WKB for WKB input,
        otherwise shapely LineStrings.
    """
    prepared = prepare(data, "explode-lines")
    return _lines(prepared, _native.lines([wkb for _, wkb in prepared.features]))
