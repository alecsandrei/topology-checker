"""Snapshot GIS inputs as planar WKB without dataset I/O or reprojection."""

from __future__ import annotations

import math
import operator
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from numbers import Real
from typing import Any, cast

import shapely
from pyproj import CRS
from pyproj.exceptions import CRSError
from shapely.geometry import (
    GeometryCollection,
    MultiLineString,
    MultiPoint,
    MultiPolygon,
    shape,
)
from shapely.geometry.base import BaseGeometry

from ._enums import GeometryKind

_FAMILIES = {
    "Point": GeometryKind.POINT,
    "MultiPoint": GeometryKind.POINT,
    "LineString": GeometryKind.LINE,
    "LinearRing": GeometryKind.LINE,
    "MultiLineString": GeometryKind.LINE,
    "Polygon": GeometryKind.POLYGON,
    "MultiPolygon": GeometryKind.POLYGON,
}


@dataclass(frozen=True)
class Prepared:
    """Owned input snapshot and original part positions."""

    features: tuple[tuple[int, bytes], ...]
    geometries: tuple[BaseGeometry, ...]
    parts: dict[int, tuple[int, ...]]
    crs: CRS | None
    warnings: tuple[str, ...]
    kind: GeometryKind | None


def resolve_crs(value: Any) -> CRS | None:
    """Parse known metadata or raise ValueError for an invalid CRS."""
    if value is None:
        return None
    try:
        if isinstance(value, Mapping):
            metadata = cast(Mapping[str, Any], value)
            if metadata.get("type") == "name":
                value = metadata["properties"]["name"]
        return CRS.from_user_input(value)  # pyright: ignore[reportUnknownMemberType]
    except (CRSError, KeyError, TypeError, ValueError) as exc:
        raise ValueError(f"invalid CRS: {value!r}") from exc


def _items(data: Any) -> tuple[list[Any], Any]:
    """Read geometry values and inferred CRS from an input adapter."""
    if hasattr(data, "geometry") and hasattr(data, "crs"):
        return list(data.geometry), data.crs
    if isinstance(data, BaseGeometry) or data is None:
        return [data], None
    if hasattr(data, "__geo_interface__"):
        metadata = getattr(data, "crs", None)
        items, embedded = _items(data.__geo_interface__)
        if metadata is not None and embedded is not None:
            if resolve_crs(metadata) != resolve_crs(embedded):
                raise ValueError("conflicting input CRS metadata")
        return items, metadata if metadata is not None else embedded
    if isinstance(data, Mapping):
        mapping_data = cast(Mapping[str, Any], data)
        kind = mapping_data.get("type")
        if kind == "FeatureCollection":
            features = mapping_data.get("features")
            if not isinstance(features, Iterable):
                raise ValueError("FeatureCollection requires features")
            return list(cast(Iterable[Any], features)), mapping_data.get("crs")
        if kind == "Feature":
            return [mapping_data.get("geometry")], mapping_data.get("crs")
        return [mapping_data], mapping_data.get("crs")
    if isinstance(data, (str, bytes)) or not isinstance(data, Iterable):
        raise TypeError("data must be geometries, a GIS object, or a GeoJSON mapping")
    return list(cast(Iterable[Any], data)), None


def _geometry(value: Any) -> BaseGeometry | None:
    """Convert a single item while distinguishing missing and invalid input."""
    if value is None:
        return None
    if isinstance(value, Real):
        if math.isnan(value):
            return None
        raise TypeError(f"unsupported geometry type: {type(value).__name__}")
    if isinstance(value, BaseGeometry):
        return value
    if hasattr(value, "__geo_interface__"):
        value = value.__geo_interface__
    if not isinstance(value, Mapping):
        raise TypeError(f"unsupported geometry type: {type(value).__name__}")
    geometry_data = cast(Mapping[str, Any], value)
    if geometry_data.get("type") == "Feature":
        return _geometry(geometry_data.get("geometry"))
    supported = {
        "Point",
        "LineString",
        "LinearRing",
        "Polygon",
        "MultiPoint",
        "MultiLineString",
        "MultiPolygon",
        "GeometryCollection",
    }
    if geometry_data.get("type") not in supported:
        raise TypeError(
            f"unsupported GeoJSON geometry type: {geometry_data.get('type')!r}"
        )
    try:
        return shape(dict(geometry_data))
    except (KeyError, TypeError, ValueError, shapely.errors.GEOSException) as exc:
        raise ValueError("invalid GeoJSON geometry") from exc


def _normalize(
    geometry: BaseGeometry,
    families: set[GeometryKind],
    hints: set[GeometryKind],
) -> tuple[BaseGeometry | None, tuple[int, ...], int]:
    """Remove empty leaves and map normalized parts to depth-first input positions."""
    positions: list[int] = []
    skipped = 0
    position = 0

    def visit(item: BaseGeometry) -> BaseGeometry | None:
        nonlocal position, skipped
        kind = _FAMILIES.get(item.geom_type)
        if item.is_empty and kind is not None:
            hints.add(kind)
        if item.geom_type in {
            "MultiPoint",
            "MultiLineString",
            "MultiPolygon",
            "GeometryCollection",
        }:
            children = [
                clean
                for child in cast(Any, item).geoms
                if (clean := visit(child)) is not None
            ]
            if not children:
                return None
            constructors: dict[str, Any] = {
                "MultiPoint": MultiPoint,
                "MultiLineString": MultiLineString,
                "MultiPolygon": MultiPolygon,
                "GeometryCollection": GeometryCollection,
            }
            return cast(BaseGeometry, constructors[item.geom_type](children))
        original = position
        position += 1
        if item.is_empty:
            skipped += 1
            return None
        if kind is not None:
            families.add(kind)
        positions.append(original)
        return item

    clean = visit(geometry)
    return clean, tuple(positions), skipped


def _fids(values: Iterable[int] | None, length: int) -> list[int]:
    """Validate FIDs including positions that will subsequently be skipped."""
    if values is None:
        return list(range(length))
    result: list[int] = []
    for value in values:
        if isinstance(value, bool):
            raise ValueError("FIDs must be signed 64-bit integers")
        try:
            fid = operator.index(value)
        except TypeError as exc:
            raise ValueError("FIDs must be signed 64-bit integers") from exc
        if not -(2**63) <= fid < 2**63:
            raise ValueError("FIDs must be signed 64-bit integers")
        result.append(fid)
    if len(result) != length:
        raise ValueError("FIDs must match input length")
    if len(set(result)) != length:
        raise ValueError("FIDs must be unique")
    return result


def prepare(
    data: Any,
    name: str,
    fids: Iterable[int] | None = None,
    crs: Any = None,
    kind: GeometryKind | None = None,
) -> Prepared:
    """Copy inputs, validate metadata, and normalize empty components.

    Args:
        data: Geometry iterable, GIS object, or GeoJSON mapping.
        name: Display name used in warnings.
        fids: Optional unique signed feature IDs for all original positions.
        crs: Optional CRS, which must agree with inferred metadata.
        kind: Optional concrete geometry family for ambiguous inputs.

    Returns:
        A complete registration snapshot, ready for the native boundary.

    Raises:
        TypeError: An input geometry, adapter, or declaration type is unsupported.
        ValueError: FIDs, coordinates, geometry data, or CRS metadata are invalid.
    """
    if kind is not None:
        if not isinstance(cast(object, kind), GeometryKind):
            raise TypeError("kind must be a GeometryKind member or None")
        if kind not in (GeometryKind.POINT, GeometryKind.LINE, GeometryKind.POLYGON):
            raise ValueError("kind must be POINT, LINE, or POLYGON")
    items, inferred = _items(data)
    explicit_crs, inferred_crs = resolve_crs(crs), resolve_crs(inferred)
    if explicit_crs is not None and inferred_crs is not None:
        if explicit_crs != inferred_crs:
            raise ValueError("explicit CRS conflicts with input CRS metadata")
    ids = _fids(fids, len(items))
    features: list[tuple[int, bytes]] = []
    snapshots: list[BaseGeometry] = []
    parts: dict[int, tuple[int, ...]] = {}
    warnings: list[str] = []
    families: set[GeometryKind] = set()
    hints: set[GeometryKind] = set()
    for position, (fid, value) in enumerate(zip(ids, items, strict=True)):
        geometry = _geometry(value)
        if geometry is None or geometry.is_empty:
            if geometry is not None:
                _normalize(geometry, families, hints)
            warnings.append(
                f"layer {name}, input {position}, FID {fid}: skipped "
                f"{'missing' if geometry is None else 'empty'} feature"
            )
            continue
        # Third-party stubs leave NumPy-style keyword arguments untyped.
        dimension = shapely.get_coordinate_dimension(geometry)  # pyright: ignore[reportUnknownMemberType]
        if int(dimension) > 2:
            warnings.append(f"layer {name}, FID {fid}: discarded extra dimensions")
        # WKB provides an immutable XY snapshot, even for custom GIS adapters.
        snapshot = shapely.from_wkb(
            shapely.to_wkb(geometry, output_dimension=2, flavor="iso")
        )
        clean, positions, skipped = _normalize(snapshot, families, hints)
        assert clean is not None
        if skipped:
            warnings.append(f"layer {name}, FID {fid}: skipped {skipped} empty parts")
        features.append((fid, shapely.to_wkb(clean, output_dimension=2)))
        snapshots.append(snapshot)
        parts[fid] = positions
    if kind is not None and families - {kind}:
        raise ValueError(
            "declared kind conflicts with retained nonempty geometry parts"
        )
    inferred_kind = (
        next(iter(families))
        if len(families) == 1
        else GeometryKind.COLLECTION
        if families
        else next(iter(hints))
        if len(hints) == 1
        else None
    )
    return Prepared(
        tuple(features),
        tuple(snapshots),
        parts,
        explicit_crs if explicit_crs is not None else inferred_crs,
        tuple(warnings),
        kind if kind is not None else inferred_kind,
    )
