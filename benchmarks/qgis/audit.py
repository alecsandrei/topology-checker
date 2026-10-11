"""Recheck correctness after adding probes, preserving all measured timings."""

import argparse
import hashlib
import json
from datetime import UTC, datetime
from pathlib import Path

from fixtures import HERE, MAPPINGS, collection, workload
from plot import publish
from run import ROOT, QgisSession, atomic_json, probes


def verify_workloads(data: dict) -> int:
    """Verify that adding probes did not change any timed workload geometry."""
    count = 0
    for record in data["rules"]:
        for measured in record["workloads"]:
            primary, secondary, bad_blocks = workload(record["rule"], measured["size"])
            if bad_blocks != measured["invalid_blocks"]:
                raise ValueError("Invalid-block count changed after measurement")
            for name, geometries in (("input", primary), ("other", secondary)):
                if geometries is None:
                    continue
                encoded = (
                    json.dumps(collection(geometries, name), separators=(",", ":"))
                    + "\n"
                ).encode()
                if (
                    hashlib.sha256(encoded).hexdigest()
                    != measured["fixture"][name]["sha256"]
                ):
                    raise ValueError(
                        f"Timed fixture changed: {record['rule']}, {measured['size']}, {name}"
                    )
            count += 1
    return count


def main() -> None:
    """Refresh selected probe suites and gates with separately recorded provenance."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path)
    parser.add_argument("--rules", nargs="+", choices=list(MAPPINGS), required=True)
    command = parser.parse_args()
    data = json.loads(command.results.read_text())
    metadata = data["metadata"]
    if "finished_at" not in metadata:
        parser.error("Wait for the measurement run to finish before auditing")
    binary = ROOT / "target/release/examples/qgis_benchmark"
    if (
        hashlib.sha256(binary.read_bytes()).hexdigest()
        != metadata["rust_binary_sha256"]
    ):
        parser.error("The Rust benchmark binary changed since measurement")
    qgis_binary = Path(metadata["qgis_executable"])
    if hashlib.sha256(qgis_binary.read_bytes()).hexdigest() != metadata["qgis_sha256"]:
        parser.error("The QGIS executable changed since measurement")
    args = argparse.Namespace(
        rust=binary,
        qgis=str(qgis_binary),
        startup_timeout=metadata["startup_timeout_s"],
        timeout=metadata["validation_timeout_s"],
    )
    audit = {
        "started_at": datetime.now(UTC).isoformat(),
        "rules": command.rules,
        "tooling_sha256": {
            p.name: hashlib.sha256(p.read_bytes()).hexdigest()
            for p in (
                HERE / "fixtures.py",
                HERE / "qgis_startup.py",
                HERE / "run.py",
                HERE / "audit.py",
            )
        },
    }
    print("Verifying all measured workload hashes", flush=True)
    audit["workload_hashes_reproduced"] = verify_workloads(data)
    directory = (
        ROOT / "builds/qgis" / datetime.now(UTC).strftime("audit-%Y%m%dT%H%M%S%f")
    )
    for rule in command.rules:
        print(f"Audit: {rule}", flush=True)
        record = next(r for r in data["rules"] if r["rule"] == rule)
        rule_directory = directory / rule.replace("/", "_")
        qgis = QgisSession(args.qgis, rule_directory / "session", args.startup_timeout)
        try:
            if qgis.metadata["plugin_sha256"] != metadata["qgis"]["plugin_sha256"]:
                raise ValueError("The installed C++ plugin changed since measurement")
            checks = probes(rule, args, rule_directory, qgis)
        finally:
            qgis.close()
        record["pre_audit_probes"] = record["probes"]
        record["probes"] = checks
        record["mapping"] = MAPPINGS[rule]
        record["correctness_passed"] = all(p["rust_expected"] for p in checks)
        record["paired_gate"] = record["correctness_passed"] and all(
            p["agrees"] is True for p in checks
        )
        for work in record["workloads"]:
            work["paired_eligible"] = (
                record["paired_gate"]
                and work["workload_agrees"]
                and not work["failures"]
                and all(
                    len(work[e]) == metadata["repetitions"]
                    and all(s["status"] == "ok" for s in work[e])
                    for e in ("rust", "qgis")
                )
            )
    audit["finished_at"] = datetime.now(UTC).isoformat()
    metadata.setdefault("correctness_audits", []).append(audit)
    atomic_json(command.results, data)
    publish(command.results, command.results.parent)


if __name__ == "__main__":
    main()
