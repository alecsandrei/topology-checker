"""Lazy access to the optional shapely and pyproj dependencies."""

from __future__ import annotations

import sys


# Return types stay inferred so callers keep the typed module and class.
def shapely():
    """Import shapely or explain which extra provides it."""
    try:
        import shapely
    except ImportError as exc:
        raise ImportError(
            "Install topology-checker[io] to use shapely geometries"
        ) from exc
    return shapely


def crs_class():
    """Import pyproj's CRS or explain which extra provides it."""
    try:
        from pyproj import CRS
    except ImportError as exc:
        raise ImportError("Install topology-checker[io] to use CRS metadata") from exc
    return CRS


def is_geometry(value: object) -> bool:
    """Detect shapely geometries without importing shapely."""
    base = sys.modules.get("shapely.geometry.base")
    return base is not None and isinstance(value, base.BaseGeometry)
