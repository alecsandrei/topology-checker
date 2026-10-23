"""Snapshot GIS inputs as planar WKB without dataset I/O or reprojection."""

from __future__ import annotations

import math
import operator
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from numbers import Real
from typing import TYPE_CHECKING, Any, cast

from . import _native, _optional
from ._enums import GeometryKind

if TYPE_CHECKING:
    from pyproj import CRS
    from shapely.geometry.base import BaseGeometry

_GEOJSON_TYPES = frozenset(
    {
        "Point",
        "LineString",
        "LinearRing",
        "Polygon",
        "MultiPoint",
        "MultiLineString",
        "MultiPolygon",
        "GeometryCollection",
    }
)


@dataclass(frozen=True)
class Prepared:
    """Owned input snapshot and original part positions.

    Parts only list features that lost empty parts; others keep native numbering.
    """

    features: tuple[tuple[int, bytes], ...]
    wkb: tuple[bytes, ...]
    parts: dict[int, tuple[int, ...]]
    crs: CRS | None
    warnings: tuple[str, ...]
    kind: GeometryKind | None
    wkb_only: bool


def resolve_crs(value: Any) -> CRS | None:
    """Parse known metadata or raise ValueError for an invalid CRS."""
    if value is None:
        return None
    crs_class = _optional.crs_class()
    from pyproj.exceptions import CRSError

    try:
        if isinstance(value, Mapping):
            metadata = cast(Mapping[str, Any], value)
            if metadata.get("type") == "name":
                value = metadata["properties"]["name"]
        return crs_class.from_user_input(value)  # pyright: ignore[reportUnknownMemberType]
    except (CRSError, KeyError, TypeError, ValueError) as exc:
        raise ValueError(f"invalid CRS: {value!r}") from exc


def _items(data: Any) -> tuple[list[Any], Any]:
    """Read geometry values and inferred CRS from an input adapter."""
    if hasattr(data, "geometry") and hasattr(data, "crs"):
        return list(data.geometry), data.crs
    if data is None or _optional.is_geometry(data):
        return [data], None
    if isinstance(data, (str, bytes, bytearray, memoryview)):
        raise TypeError(
            "data must be geometries, WKB items, a GIS object, or a GeoJSON mapping"
        )
    if hasattr(data, "__geo_interface__"):
        metadata = getattr(data, "crs", None)
        items, embedded = _items(data.__geo_interface__)
        if metadata is not None and embedded is not None:
            metadata_crs, embedded_crs = resolve_crs(metadata), resolve_crs(embedded)
            if metadata_crs is None or not metadata_crs.equals(
                embedded_crs, ignore_axis_order=True
            ):
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
    if not isinstance(data, Iterable):
        raise TypeError(
            "data must be geometries, WKB items, a GIS object, or a GeoJSON mapping"
        )
    return list(cast(Iterable[Any], data)), None


def _geometry(value: Any) -> BaseGeometry | None:
    """Convert a non-WKB item while distinguishing missing and invalid input."""
    if value is None:
        return None
    if isinstance(value, Real):
        if math.isnan(value):
            return None
        raise TypeError(f"unsupported geometry type: {type(value).__name__}")
    if _optional.is_geometry(value):
        return cast("BaseGeometry", value)
    if hasattr(value, "__geo_interface__"):
        value = value.__geo_interface__
    if not isinstance(value, Mapping):
        raise TypeError(f"unsupported geometry type: {type(value).__name__}")
    geometry_data = cast(Mapping[str, Any], value)
    if geometry_data.get("type") == "Feature":
        return _geometry(geometry_data.get("geometry"))
    if geometry_data.get("type") not in _GEOJSON_TYPES:
        raise TypeError(
            f"unsupported GeoJSON geometry type: {geometry_data.get('type')!r}"
        )
    shapely = _optional.shapely()
    try:
        return shapely.geometry.shape(dict(geometry_data))
    except (KeyError, TypeError, ValueError, shapely.errors.GEOSException) as exc:
        raise ValueError("invalid GeoJSON geometry") from exc


def _wkb_items(items: list[Any]) -> tuple[list[bytes | None], bool]:
    """Read WKB items as bytes and encode other geometries as ISO WKB.

    Returns:
        One WKB value or None (missing) per input, and whether every present
        input was already WKB.
    """
    values: list[bytes | None] = []
    pending: list[tuple[int, BaseGeometry]] = []
    for index, value in enumerate(items):
        if isinstance(value, bytes):
            values.append(value)
        elif isinstance(value, (bytearray, memoryview)):
            values.append(bytes(cast("bytearray | memoryview[int]", value)))
        else:
            values.append(None)
            if (geometry := _geometry(value)) is not None:
                pending.append((index, geometry))
    if pending:
        shapely = _optional.shapely()
        # Three output dimensions keep a Z or M flag, which triggers the warning.
        encoded = cast(
            list[bytes],
            shapely.to_wkb(  # pyright: ignore[reportUnknownMemberType]
                [geometry for _, geometry in pending],
                output_dimension=3,
                flavor="iso",
            ).tolist(),
        )
        for (index, _), wkb in zip(pending, encoded, strict=True):
            values[index] = wkb
    return values, not pending


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
        data: WKB items, geometry iterable, GIS object, or GeoJSON mapping.
        name: Display name used in warnings.
        fids: Optional unique signed feature IDs for all original positions.
        crs: Optional CRS, which must agree with inferred metadata.
        kind: Optional concrete geometry family for ambiguous inputs.

    Returns:
        A complete registration snapshot, ready for the native boundary.

    Raises:
        TypeError: An input geometry, adapter, or declaration type is unsupported.
        ValueError: FIDs, WKB, geometry data, or CRS metadata are invalid.
        ImportError: Shapely or CRS input needs topology-checker[io].
    """
    if kind is not None:
        if not isinstance(cast(object, kind), GeometryKind):
            raise TypeError("kind must be a GeometryKind member or None")
        if kind not in (GeometryKind.POINT, GeometryKind.LINE, GeometryKind.POLYGON):
            raise ValueError("kind must be POINT, LINE, or POLYGON")
    items, inferred = _items(data)
    explicit_crs, inferred_crs = resolve_crs(crs), resolve_crs(inferred)
    if explicit_crs is not None and inferred_crs is not None:
        if not explicit_crs.equals(inferred_crs, ignore_axis_order=True):
            raise ValueError("explicit CRS conflicts with input CRS metadata")
    ids = _fids(fids, len(items))
    values, wkb_only = _wkb_items(items)
    features, snapshots, parts, warnings, family_names, hint_names = _native.prepare(
        name, ids, values
    )
    families = {GeometryKind(value) for value in family_names}
    hints = {GeometryKind(value) for value in hint_names}
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
        {fid: tuple(positions) for fid, positions in parts},
        explicit_crs if explicit_crs is not None else inferred_crs,
        tuple(warnings),
        kind if kind is not None else inferred_kind,
        wkb_only,
    )
