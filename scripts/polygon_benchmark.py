"""Measure an isolated polygon acceptance workload using immutable WKB."""

from __future__ import annotations

import argparse
import hashlib
import json
import resource
import signal
import sys
import time
from pathlib import Path
from typing import Any

import numpy as np
import shapely
from topology_checker import Rule, Topology

WORKLOADS = {
    "bgt": ("pg_nl_bgt_begroeid_enschede", None, "MUST_NOT_OVERLAP", 0.001),
    "cbs": ("pg_nl_cbs_bodemgebruik_2015_enschede", None, "MUST_NOT_HAVE_GAPS", 0.001),
    "vienna": ("pg_vienna_realnutzung_2022", None, "MUST_NOT_OVERLAP", 0.001),
    "ons_overlap": (
        "pg_ons_lad_2021_bgc",
        "pg_ons_lad_2020_bfc",
        "MUST_NOT_OVERLAP",
        1.0,
    ),
    "ons_gaps": ("pg_ons_lad_2020_bfc", None, "MUST_NOT_HAVE_GAPS", 1.0),
    "brp": ("pg_nl_brp_gewaspercelen_enschede", None, "MUST_NOT_HAVE_GAPS", 0.001),
}


def prepared(path: Path) -> Any:
    """Read the existing cache without importing campaign cache writers."""
    with np.load(path, allow_pickle=False) as data:
        blob = data["blob"].tobytes()
        offsets = data["offsets"]
        rows = [
            None if missing else blob[offsets[i] : offsets[i + 1]]
            for i, missing in enumerate(data["missing"])
        ]
    return shapely.from_wkb(np.asarray(rows, dtype=object))


def main() -> None:
    """Separate native ingestion and validation and emit raw machine evidence."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workload", choices=[*WORKLOADS, "ingestion"])
    parser.add_argument(
        "--campaign", type=Path, default=Path("../topology-checker-test")
    )
    parser.add_argument("--equivalence", action="store_true")
    args = parser.parse_args()
    signal.alarm(900)
    hashes: dict[str, str] = {}
    if args.workload == "ingestion":
        theta = np.linspace(0, 2 * np.pi, 200_000, endpoint=False)
        parts = [
            shapely.Polygon(
                list(zip(1000 * np.cos(theta), 1000 * np.sin(theta), strict=True))
            )
        ]
        for k in range(400):
            x, y = 800 + k % 20 * 9.0, 800 + k // 20 * 9.0
            if x * x + y * y > 1000**2 * 1.01:
                parts.append(shapely.box(x, y, x + 4, y + 4))
        inputs = shapely.MultiPolygon(parts)
        assert inputs.is_valid
        a, b, rule, tolerance = [inputs], None, None, 0.001
    else:
        ak, bk, rule, tolerance = WORKLOADS[args.workload]
        root = args.campaign / "data/prepared"
        for key in [ak, *([bk] if bk else [])]:
            hashes[key] = hashlib.sha256((root / f"{key}.npz").read_bytes()).hexdigest()
        a = prepared(root / f"{ak}.npz")
        b = None if bk is None else prepared(root / f"{bk}.npz")
    topology = Topology(tolerance=tolerance)
    started = time.perf_counter()
    layer = topology.add_layer(a)
    other = None if b is None else topology.add_layer(b)
    ingested = time.perf_counter()
    if args.workload == "ingestion":
        assert ingested - started <= 2.0, (
            "multipart ingestion exceeded the two-second gate"
        )
    count = None
    if rule is not None:
        topology.add_rule(getattr(Rule, rule), layer, other=other)
        started_validation = time.perf_counter()
        report = topology.validate()
        validated = time.perf_counter()
        count = report.error_count
        assert all(v.geometry.is_valid for v in report.violations)
        if args.equivalence:
            assert report.to_dict() == topology.validate().to_dict()
            with topology.iter_validate(128) as stream:
                assert [v.to_dict() for v in stream] == [
                    v.to_dict() for v in report.violations
                ]
                assert (
                    stream.summary is not None and stream.summary.error_count == count
                )
    else:
        started_validation = validated = ingested
    print(
        json.dumps(
            {
                "workload": args.workload,
                "ingest_s": ingested - started,
                "validate_s": validated - started_validation,
                "violations": count,
                "peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
                "source_sha256": hashes,
                "package": sys.modules["topology_checker"].__file__,
                "equivalence": args.equivalence,
            }
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()
