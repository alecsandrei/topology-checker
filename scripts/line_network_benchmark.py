"""Measure sparse line-network validation in an isolated release process."""

from __future__ import annotations

import argparse
import json
import resource
import time
from typing import cast

import numpy as np
import shapely
from shapely.geometry.base import BaseGeometry
from topology_checker import Rule, Topology


def main() -> None:
    """Validate Delaunay edges that meet only at their original endpoints."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--points", type=int, default=8000)
    parser.add_argument("--overlap", action="store_true")
    parser.add_argument("--binary", action="store_true")
    parser.add_argument("--equivalence", action="store_true")
    args = parser.parse_args()
    points = shapely.MultiPoint(
        np.random.default_rng(7).uniform(0, 1000, (args.points, 2))
    )
    lines = cast(
        list[BaseGeometry],
        list(shapely.get_parts(shapely.delaunay_triangles(points, only_edges=True))),
    )
    topology = Topology(tolerance=0.001)
    started = time.perf_counter()
    source = topology.add_layer(lines[::2] if args.binary else lines)
    target = topology.add_layer(lines[1::2]) if args.binary else None
    topology.add_rule(
        Rule.MUST_NOT_OVERLAP if args.overlap else Rule.MUST_NOT_INTERSECT,
        source,
        other=target,
    )
    ingested = time.perf_counter()
    report = topology.validate()
    validated = time.perf_counter()
    assert report.is_valid, report.error_count
    if args.equivalence:
        assert topology.validate().to_dict() == report.to_dict()
        with topology.iter_validate(128) as stream:
            assert list(stream) == []
            assert stream.summary is not None and stream.summary.error_count == 0
    print(
        json.dumps(
            {
                "workload": "line_network",
                "points": args.points,
                "lines": len(lines),
                "rule": "overlap" if args.overlap else "intersect",
                "binary": args.binary,
                "ingest_s": ingested - started,
                "validate_s": validated - ingested,
                "violations": report.error_count,
                "peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
                "equivalence": args.equivalence,
            }
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()
