"""Audit campaign gap counts using independent exact source-boundary reconstruction."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
import time
from pathlib import Path
from typing import Any, cast

import numpy as np
import shapely
from shapely.geometry import Polygon, box

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from scripts.campaign_benchmark import prepared
from tests.fraction_reference import (
    Locator,
    point,
    polygons,
    signed_area,
    substantial,
    union_boundary,
)


def main() -> None:
    """Record every exact hole and every GEOS diagnostic ring, including zero area."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--campaign", type=Path, default=Path("../topology-checker-test")
    )
    parser.add_argument(
        "--output", type=Path, default=Path("docs/polygon-precision-audit.json")
    )
    parser.add_argument("--repair", action="store_true")
    args = parser.parse_args()
    path = args.campaign / "data/prepared/pg_tz_boundary_builder_oceans.npz"
    inputs = prepared(path)
    original_ids = np.arange(len(inputs))
    if not args.repair:
        original_ids = original_ids[shapely.is_valid(inputs)]
    inputs = (
        cast(Any, shapely.make_valid(inputs))
        if args.repair
        else inputs[shapely.is_valid(inputs)]
    )
    retained = cast(Any, ~(shapely.is_missing(inputs) | shapely.is_empty(inputs)))
    inputs = inputs[retained]
    original_ids = original_ids[retained]
    source_polygons: list[dict[str, Any]] = []

    def register_sources(g: Any, fid: int, path: list[int]) -> None:
        if isinstance(g, Polygon):
            source_polygons.append(
                {
                    "layer_fid": fid,
                    "original_row": int(original_ids[fid]),
                    "part_path": path,
                }
            )
        elif hasattr(g, "geoms"):
            for part, child in enumerate(g.geoms):
                register_sources(child, fid, [*path, part])

    for fid, g in enumerate(inputs):
        register_sources(g, fid, [])
    started = time.perf_counter()
    reference = union_boundary(inputs)
    exact_rings: list[dict[str, Any]] = []
    for ring in reference.holes:
        sources = [
            reference.evidence[(b, a)] for a, b in zip(ring, ring[1:], strict=False)
        ]
        exact_rings.append(
            {
                "area": str(signed_area(ring)),
                "nodes": [[str(x), str(y)] for x, y in ring],
                "source_intervals": sources,
            }
        )
    vertices: dict[tuple[float, float], list[int]] = {}
    for i, ring in enumerate(reference.holes):
        for x, y in ring:
            vertices.setdefault((float(x), float(y)), []).append(i)
    diagnostics: list[dict[str, Any]] = []
    if args.repair:
        diagnostic_sources = list(polygons(inputs))
        diagnostic_tree = shapely.STRtree(diagnostic_sources)
        diagnostic_locations: dict[int, Locator] = {}
        for p in shapely.get_parts(shapely.union_all(inputs)):
            for r in p.interiors:
                coords = [(float(x), float(y)) for x, y in r.coords]
                ids = sorted({i for v in coords for i in vertices.get(v, [])})
                midpoint_evidence: list[dict[str, Any]] = []
                for aa, bb in zip(coords, coords[1:], strict=False):
                    a, b = point(*aa), point(*bb)
                    midpoint = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2)
                    normal = (a[1] - b[1], b[0] - a[0])
                    x, y = float(midpoint[0]), float(midpoint[1])
                    left: list[int] = []
                    right: list[int] = []
                    for raw in diagnostic_tree.query(
                        box(
                            math.nextafter(x, -math.inf),
                            math.nextafter(y, -math.inf),
                            math.nextafter(x, math.inf),
                            math.nextafter(y, math.inf),
                        )
                    ):
                        j = int(raw)
                        if j not in diagnostic_locations:
                            diagnostic_locations[j] = Locator(diagnostic_sources[j])
                        locator = diagnostic_locations[j]
                        if locator.side(midpoint, normal):
                            left.append(j)
                        if locator.side(midpoint, (-normal[0], -normal[1])):
                            right.append(j)
                    midpoint_evidence.append(
                        {
                            "left_covered_by_source_polygons": sorted(left),
                            "right_covered_by_source_polygons": sorted(right),
                        }
                    )
                diagnostics.append(
                    {
                        "midpoint_side_source_evidence": midpoint_evidence,
                        "geos_area": Polygon(r).area,
                        "binary_coordinate_exact_area": str(
                            abs(signed_area([point(x, y) for x, y in coords]))
                        ),
                        "wkt": r.wkt,
                        "reference_rings_sharing_vertices": ids,
                    }
                )
    result = {
        "source_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "repair": args.repair,
        "algorithm": (
            "Fraction source intersections and infinitesimal side classification "
            "against original polygons; independent empty-side boundary graph"
        ),
        "source_polygon_context": source_polygons,
        "source_interval_fields": [
            "source_polygon_index",
            "ring_index",
            "segment_index",
        ],
        "holes_campaign_tolerance": sum(
            substantial(ring, 1.0) for ring in reference.holes
        ),
        "shells": len(reference.shells),
        "holes": len(reference.holes),
        "input_segments": reference.input_segments,
        "candidate_segments": reference.candidate_segments,
        "reference_s": time.perf_counter() - started,
        "reference_holes": exact_rings,
        "geos_diagnostic_rings": diagnostics,
    }
    if not args.repair:
        result["reference_holes"] = [
            {"area": ring["area"], "boundary_intervals": len(ring["source_intervals"])}
            for ring in exact_rings
        ]
        result["evidence_scope"] = (
            "Full graph count and exact areas; detailed repaired audit is separate."
        )
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                k: v
                for k, v in result.items()
                if k
                not in {
                    "reference_holes",
                    "geos_diagnostic_rings",
                    "source_polygon_context",
                }
            }
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()
