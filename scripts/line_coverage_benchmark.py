"""Measure isolated coverage of long routes against an original polygon union."""

from __future__ import annotations

import argparse
import json
import resource
import time

import numpy as np
import shapely
from topology_checker import Rule, Topology


def main() -> None:
    """Check independent geometry, the two-second gate and optional stream equality."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--equivalence", action="store_true")
    args = parser.parse_args()
    xs = np.linspace(0, 1000, 8000)
    lines = [
        shapely.LineString(list(zip(xs, 10 * i + np.sin(xs), strict=True)))
        for i in range(10)
    ]
    polygons = [shapely.box(x, -5, x + 3, 100) for x in np.linspace(0, 990, 40)]
    topology = Topology(tolerance=0.001)
    started = time.perf_counter()
    a = topology.add_layer(lines)
    b = topology.add_layer(polygons)
    ingested = time.perf_counter()
    topology.add_rule(Rule.MUST_BE_INSIDE, a, other=b)
    started_validation = time.perf_counter()
    report = topology.validate()
    validated = time.perf_counter()
    assert validated - started_validation <= 2.0
    assert report.error_count == len(lines)
    union = shapely.union_all(polygons)
    for v in report.violations:
        assert v.origin is not None and v.destination is None
        expected = lines[v.origin.fid].difference(union)
        assert v.geometry.is_valid
        assert v.geometry.hausdorff_distance(expected) < 1e-8
        assert abs(v.geometry.length - expected.length) < expected.length * 1e-10
    if args.equivalence:
        assert report.to_dict() == topology.validate().to_dict()
        with topology.iter_validate(128) as stream:
            assert [v.to_dict() for v in stream] == [
                v.to_dict() for v in report.violations
            ]
            assert stream.summary is not None and stream.summary.error_count == 10
    print(
        json.dumps(
            {
                "workload": "line_inside",
                "ingest_s": ingested - started,
                "validate_s": validated - started_validation,
                "violations": report.error_count,
                "peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
                "equivalence": args.equivalence,
            }
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()
