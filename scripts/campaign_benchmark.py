"""Run one isolated campaign acceptance workload without changing campaign files."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import resource
import signal
import sys
import threading
import time
import warnings
from pathlib import Path
from typing import Any, cast

import numpy as np
import shapely
from shapely.geometry import LineString, Point, Polygon
from topology_checker import ErrorType, Report, Rule, Topology


def prepared(path: Path) -> Any:
    """Read the campaign's concatenated WKB cache without importing its writers."""
    with np.load(path, allow_pickle=False) as data:
        blob = data["blob"].tobytes()
        offsets = data["offsets"]
        missing = data["missing"]
        rows = [
            None if missing[i] else blob[offsets[i] : offsets[i + 1]]
            for i in range(len(missing))
        ]
    return shapely.from_wkb(np.asarray(rows, dtype=object))


def main() -> None:
    """Print machine-readable timings, counts and process peak RSS."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["lines", "points", "duplicates", "gaps"])
    parser.add_argument("--tolerance", type=float)
    parser.add_argument("--size", type=int, default=100_000)
    parser.add_argument("--queries", type=int, default=2000)
    parser.add_argument("--details", action="store_true")
    parser.add_argument("--repair", action="store_true")
    parser.add_argument("--equivalence", action="store_true")
    parser.add_argument(
        "--campaign", type=Path, default=Path("../topology-checker-test")
    )
    args = parser.parse_args()
    try:
        resource.setrlimit(resource.RLIMIT_AS, (32 * 1024**3, 32 * 1024**3))
    except ValueError:
        # Darwin does not implement a usable address-space limit. Enforce RSS
        # externally so the campaign cannot exhaust the workstation.
        def monitor() -> None:
            while True:
                rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
                if rss > 32 * 1024**3:
                    os._exit(137)
                time.sleep(0.5)

        threading.Thread(target=monitor, daemon=True).start()
    signal.alarm(900)
    print(
        f"package: {sys.modules['topology_checker'].__file__}",
        file=sys.stderr,
        flush=True,
    )
    started = time.perf_counter()
    tolerance = (
        args.tolerance
        if args.tolerance is not None
        else (1.0 if args.mode == "gaps" else 0.0)
    )
    topology = Topology(tolerance=tolerance)
    path = args.campaign / "data/prepared/pg_tz_boundary_builder_oceans.npz"
    reference: Any = None
    report: Report | None = None
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        if args.mode == "lines":
            # Twenty original coordinates per separated feature.
            geoms = [
                LineString([(i * 32 + j, j % 2) for j in range(20)])
                for i in range(args.size)
            ]
            layer = topology.add_layer(geoms)
            topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, layer)
        elif args.mode == "duplicates":
            layer = topology.add_layer([Point(0, 0)] * args.size)
            topology.add_rule(Rule.MUST_NOT_OVERLAP, layer)
        elif args.mode == "points":
            theta = np.linspace(0, 2 * np.pi, args.size, endpoint=False)
            polygon = Polygon(
                list(zip(10000 * np.cos(theta), 10000 * np.sin(theta), strict=True))
            )
            layer = topology.add_layer([polygon])
            points = topology.add_layer(
                [Point(i % 100 - 50, i // 100 % 100 - 50) for i in range(args.queries)]
            )
            topology.add_rule(Rule.MUST_NOT_BE_CONTAINED_BY, points, other=layer)
        else:
            path = args.campaign / "data/prepared/pg_tz_boundary_builder_oceans.npz"
            geoms = prepared(path)
            valid: Any = cast(Any, shapely.is_valid(geoms))
            if args.repair:
                geoms = cast(Any, shapely.make_valid(geoms))
            else:
                geoms = geoms[valid]
            geoms = geoms[~(shapely.is_missing(geoms) | shapely.is_empty(geoms))]
            layer = topology.add_layer(geoms)
            topology.add_rule(Rule.MUST_NOT_HAVE_GAPS, layer)
            reference = geoms
        prepared_at = time.perf_counter()
        if args.mode == "duplicates":
            count = 0
            with topology.iter_validate(128) as stream:
                for _ in stream:
                    count += 1
            assert count == args.size * (args.size - 1) // 2
        else:
            report = topology.validate()
            count = report.error_count
        validated_at = time.perf_counter()
        if args.mode == "points":
            assert count == args.queries
            topology.validate()
        if reference is not None:
            assert report is not None
            union = shapely.union_all(reference)
            parts = shapely.get_parts(union)
            shells = len(parts)
            # Acceptance counts come from the independently reconstructed exact
            # source graph. GEOS remains a geometry diagnostic, not a count oracle.
            audit_path = (
                Path(__file__).resolve().parents[1]
                / "docs"
                / (
                    "polygon-precision-audit.json"
                    if args.repair
                    else "polygon-precision-valid-audit.json"
                )
            )
            audit = json.loads(audit_path.read_text())
            assert (
                audit["source_sha256"] == hashlib.sha256(path.read_bytes()).hexdigest()
            )
            shells = audit["shells"]
            if tolerance == 0:
                holes = audit["holes"]
            elif tolerance == 1:
                holes = audit["holes_campaign_tolerance"]
            else:
                sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
                from tests.fraction_reference import substantial, union_boundary

                holes = sum(
                    substantial(ring, tolerance)
                    for ring in union_boundary(reference).holes
                )
            if args.details:
                Path("/tmp/topology-reference.wkb").write_bytes(union.wkb)
                Path("/tmp/topology-native-rings.json").write_text(
                    json.dumps(
                        [
                            {"kind": v.error_type.value, "wkt": v.geometry.wkt}
                            for v in report.violations
                        ]
                    )
                )
                print(
                    json.dumps(
                        {
                            "prepare_s": prepared_at - started,
                            "validate_s": validated_at - prepared_at,
                            "rss": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
                        }
                    ),
                    flush=True,
                )
            if args.equivalence:
                assert topology.validate().to_dict() == report.to_dict()
                with topology.iter_validate(128) as stream:
                    streamed = list(stream)
                assert [v.to_dict() for v in streamed] == [
                    v.to_dict() for v in report.violations
                ]
                assert (
                    stream.summary is not None and stream.summary.error_count == count
                )
            assert all(
                v.origin is None and v.destination is None for v in report.violations
            )
            counts = report.rules[0].counts
            assert counts.get(ErrorType.OUTER_BOUNDARY, 0) == shells, (counts, shells)
            assert counts.get(ErrorType.GAP, 0) == holes, (counts, holes)
            assert all(
                v.geometry.is_valid and v.geometry.is_ring for v in report.violations
            )
    print(
        json.dumps(
            {
                "mode": args.mode,
                "size": args.size,
                "queries": args.queries,
                "repair": args.repair,
                "tolerance": tolerance,
                "prepare_s": prepared_at - started,
                "validate_s": validated_at - prepared_at,
                "total_s": time.perf_counter() - started,
                "violations": count,
                "peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
            }
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()
