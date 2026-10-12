"""Independent union boundary reference, using Fraction and original source intervals.

GEOS supplies WKB parsing and bounding-box indexes only. No overlay, predicates,
polygonization, or Rust boundary traversal determines the reference result.
"""

from __future__ import annotations

import math
from collections import defaultdict
from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from fractions import Fraction
from functools import cmp_to_key, lru_cache
from math import isqrt
from typing import Any, cast

import numpy as np
import shapely
from shapely.geometry import LineString, Polygon, box
from shapely.geometry.base import BaseGeometry

Point = tuple[Fraction, Fraction]

# NumPy accelerates bounding-box and binary-identity bookkeeping only. Exact
# geometry and symbolic classifications below have explicit Fraction types.
arrays = cast(Any, np)


@lru_cache(maxsize=250_000)
def point(x: float, y: float) -> Point:
    """Convert original binary coordinates exactly."""
    return Fraction(x), Fraction(y)


def cross(a: Point, b: Point, c: Point) -> Fraction:
    """Return the exact orientation determinant."""
    return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])


def signed_area(ring: Sequence[Point]) -> Fraction:
    """Compute the exact area, with no floating-point operations."""
    return (
        sum(
            (a[0] * b[1] - a[1] * b[0] for a, b in zip(ring, ring[1:], strict=False)),
            Fraction(),
        )
        / 2
    )


def intersections(a: Point, b: Point, c: Point, d: Point) -> list[Point]:
    """Node two source segments using exact rational arithmetic."""
    determinant = cross(
        (Fraction(), Fraction()), (b[0] - a[0], b[1] - a[1]), (d[0] - c[0], d[1] - c[1])
    )
    if determinant:
        t = cross(c, d, a) / determinant
        u = cross(a, b, c) / -determinant
        if 0 <= t <= 1 and 0 <= u <= 1:
            return [(a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]))]
        return []
    if cross(a, b, c):
        return []
    return [
        p
        for p in {a, b, c, d}
        if min(a, b) <= p <= max(a, b) and min(c, d) <= p <= max(c, d)
    ]


def polygons(geometries: Iterable[BaseGeometry]) -> Iterable[Polygon]:
    """Flatten input containers while retaining polygon input coordinates."""
    for g in geometries:
        if isinstance(g, Polygon):
            yield g
        elif hasattr(g, "geoms"):
            yield from polygons(cast(Any, g).geoms)


def ring_sign(coords: Any) -> int:
    """Certify orientation in binary arithmetic, with exact cancellation fallback."""
    a = arrays.asarray(coords, dtype=float)
    d = a - a[0]
    lower = arrays.nextafter(d, -math.inf)
    upper = arrays.nextafter(d, math.inf)

    def product(ax: Any, bx: Any, ay: Any, by: Any) -> tuple[Any, Any]:
        terms = arrays.stack([ax * ay, ax * by, bx * ay, bx * by])
        return arrays.nextafter(terms.min(axis=0), -math.inf), arrays.nextafter(
            terms.max(axis=0), math.inf
        )

    lo1, hi1 = product(lower[:-1, 0], upper[:-1, 0], lower[1:, 1], upper[1:, 1])
    lo2, hi2 = product(lower[:-1, 1], upper[:-1, 1], lower[1:, 0], upper[1:, 0])
    lo = math.nextafter(math.fsum(arrays.nextafter(lo1 - hi2, -math.inf)), -math.inf)
    hi = math.nextafter(math.fsum(arrays.nextafter(hi1 - lo2, math.inf)), math.inf)
    if lo > 0:
        return 1
    if hi < 0:
        return -1
    exact = signed_area([point(x, y) for x, y in a])
    assert exact
    return 1 if exact > 0 else -1


class Locator:
    """Classify infinitesimal sides of an interval against an original polygon."""

    def __init__(self, polygon: Polygon) -> None:
        self.segments = [
            ((a[0], a[1]), (b[0], b[1]))
            for r in [polygon.exterior, *polygon.interiors]
            for a, b in zip(r.coords, list(r.coords)[1:], strict=False)
        ]
        self.tree = shapely.STRtree([LineString(s) for s in self.segments])
        self.maxx = polygon.bounds[2]

    def side(self, p: Point, normal: Point) -> bool:
        """Use lexicographic constant and epsilon signs, without choosing epsilon."""
        x, y = float(p[0]), float(p[1])
        candidates = self.tree.query(
            box(
                math.nextafter(x, -math.inf),
                math.nextafter(y, -math.inf),
                max(self.maxx, x),
                math.nextafter(y, math.inf),
            )
        )
        inside = False
        for raw in candidates:
            aa, bb = self.segments[int(raw)]
            a, b = point(*aa), point(*bb)
            ay = (a[1] - p[1], -normal[1]) > (Fraction(), Fraction())
            by = (b[1] - p[1], -normal[1]) > (Fraction(), Fraction())
            determinant = cross(a, b, p)
            derivative = (b[0] - a[0]) * normal[1] - (b[1] - a[1]) * normal[0]
            ccw = (determinant, derivative) > (Fraction(), Fraction())
            if ay != by and ccw == (b[1] > a[1]):
                inside = not inside
        return inside


@dataclass
class BoundaryReference:
    """Exact empty-region cycles and the original sources of their edges."""

    holes: list[list[Point]]
    shells: list[list[Point]]
    evidence: dict[tuple[Point, Point], tuple[int, int, int]]
    input_segments: int
    candidate_segments: int


def union_boundary(geometries: Iterable[BaseGeometry]) -> BoundaryReference:
    """Reconstruct a union boundary independently from classified source intervals."""
    inputs = list(polygons(geometries))
    if not inputs:
        return BoundaryReference([], [], {}, 0, 0)
    chunks: list[Any] = []
    source_chunks: list[Any] = []
    signs: list[Any] = []
    for fid, p in enumerate(inputs):
        for ring_id, r in enumerate([p.exterior, *p.interiors]):
            a = arrays.asarray(r.coords)[:, :2]
            e = arrays.concatenate([a[:-1], a[1:]], axis=1)
            sign = ring_sign(a) * (1 if ring_id == 0 else -1)
            forward = (e[:, 0] < e[:, 2]) | ((e[:, 0] == e[:, 2]) & (e[:, 1] < e[:, 3]))
            e[~forward] = e[~forward][:, [2, 3, 0, 1]]
            chunks.append(e)
            signs.append(arrays.where(forward, sign, -sign))
            source_chunks.append(
                arrays.column_stack(
                    [
                        arrays.full(len(e), fid),
                        arrays.full(len(e), ring_id),
                        arrays.arange(len(e)),
                    ]
                )
            )
    rows = arrays.concatenate(chunks)
    directions = arrays.concatenate(signs)
    sources = arrays.concatenate(source_chunks)
    del chunks, signs, source_chunks
    # Oppositely directed, identical original intervals have covered area on
    # both sides. They cannot be a union boundary, even at tolerance zero.
    unique, first, inverse = arrays.unique(
        rows, axis=0, return_index=True, return_inverse=True
    )
    positive = arrays.bincount(inverse, weights=directions > 0)
    negative = arrays.bincount(inverse, weights=directions < 0)
    retained = (positive == 0) | (negative == 0)
    candidate_rows = unique[retained]
    candidate_sources = sources[first[retained]]
    candidate_directions = arrays.where(positive[retained] > 0, 1, -1)
    input_segments = len(rows)
    del rows, directions, sources, unique, first, inverse, positive, negative
    print(
        f"reference: {input_segments} source segments, "
        f"{len(candidate_rows)} uncancelled candidates",
        flush=True,
    )
    edges = [(point(*r[:2]), point(*r[2:])) for r in candidate_rows]
    tree = shapely.STRtree([LineString(r.reshape(2, 2)) for r in candidate_rows])
    cuts = [{a, b} for a, b in edges]
    for i, (a, b) in enumerate(edges):
        for raw in tree.query(box(*LineString(candidate_rows[i].reshape(2, 2)).bounds)):
            j = int(raw)
            if j <= i:
                continue
            c, d = edges[j]
            contacts = intersections(a, b, c, d)
            cuts[i].update(contacts)
            cuts[j].update(contacts)
    input_tree = shapely.STRtree(inputs)
    locators: dict[int, Locator] = {}
    boundary: dict[tuple[Point, Point], tuple[int, int, int]] = {}
    for i, cut in enumerate(cuts):
        fid, ring_id, segment = map(int, candidate_sources[i])
        sequence = sorted(cut)
        for a, b in zip(sequence, sequence[1:], strict=False):
            midpoint = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2)
            normal = (a[1] - b[1], b[0] - a[0])
            left, right = candidate_directions[i] > 0, candidate_directions[i] < 0
            x, y = float(midpoint[0]), float(midpoint[1])
            for raw in input_tree.query(
                box(
                    math.nextafter(x, -math.inf),
                    math.nextafter(y, -math.inf),
                    math.nextafter(x, math.inf),
                    math.nextafter(y, math.inf),
                )
            ):
                j = int(raw)
                if j == fid:
                    continue
                if j not in locators:
                    locators[j] = Locator(inputs[j])
                locator = locators[j]
                left = left or locator.side(midpoint, normal)
                right = right or locator.side(midpoint, (-normal[0], -normal[1]))
                if left and right:
                    break
            if left != right:
                key = (a, b) if left else (b, a)
                boundary[key] = (fid, ring_id, segment)
    print(f"reference: {len(boundary)} classified boundary intervals", flush=True)
    # Walk empty-side half edges in this independently classified graph. Exact
    # angular sorting handles contacts. Stack decomposition separates lobes that
    # meet at an articulation point, so each bounded empty region is counted.
    neighbors: dict[Point, list[Point]] = defaultdict(list)
    for a, b in boundary:
        neighbors[a].append(b)
        neighbors[b].append(a)

    def angle(o: Point, a: Point, b: Point) -> int:
        u, v = (a[0] - o[0], a[1] - o[1]), (b[0] - o[0], b[1] - o[1])
        ha, hb = (
            (u[1] < 0 or u[1] == 0 and u[0] < 0),
            (v[1] < 0 or v[1] == 0 and v[0] < 0),
        )
        if ha != hb:
            return 1 if ha else -1
        determinant = u[0] * v[1] - u[1] * v[0]
        return -1 if determinant > 0 else 1 if determinant < 0 else 0

    for o, values in neighbors.items():
        values.sort(key=cmp_to_key(lambda a, b, o=o: angle(o, a, b)))
    visited: set[tuple[Point, Point]] = set()
    holes: list[list[Point]] = []
    shells: list[list[Point]] = []
    for start in boundary:
        edge = (start[1], start[0])
        if edge in visited:
            continue
        walk = [edge[0]]
        while edge not in visited:
            visited.add(edge)
            a, b = edge
            walk.append(b)
            values = neighbors[b]
            edge = (b, values[(values.index(a) - 1) % len(values)])
            assert (edge[1], edge[0]) in boundary
        path: list[Point] = []
        positions: dict[Point, int] = {}
        for p in walk:
            if p in positions:
                index = positions[p]
                cycle = [p, *path[index + 1 :], p]
                for v in path[index + 1 :]:
                    positions.pop(v)
                del path[index + 1 :]
                area = signed_area(cycle)
                if area > 0:
                    holes.append(cycle)
                elif area < 0:
                    shells.append(cycle)
            else:
                positions[p] = len(path)
                path.append(p)
    return BoundaryReference(
        holes, shells, boundary, input_segments, len(candidate_rows)
    )


def substantial(ring: Sequence[Point], tolerance: float) -> bool:
    """Certify the inclusive area-to-perimeter threshold on exact reference nodes."""
    area = abs(signed_area(ring))
    if not area:
        return False
    if tolerance == 0:
        return True
    threshold = 2 * area / Fraction(tolerance)
    squares = [
        (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2
        for a, b in zip(ring, ring[1:], strict=False)
    ]
    bits = 64
    while True:
        scale = 1 << bits
        lower, upper = Fraction(), Fraction()
        for square in squares:
            root = isqrt((square.numerator << (bits * 2)) // square.denominator)
            lo = Fraction(root, scale)
            lower += lo
            upper += lo if lo**2 == square else Fraction(root + 1, scale)
        if upper <= threshold:
            return True
        if lower > threshold:
            return False
        bits *= 2
