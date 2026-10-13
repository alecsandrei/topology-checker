"""Run sequential GIS comparisons with correctness gates and full provenance."""

import argparse
import hashlib
import os
import platform
import shutil
import subprocess
from contextlib import ExitStack
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import psycopg

from benchmarks.gis.fixtures import CATALOG, FIXTURES, ROOT, workload, write_case
from benchmarks.gis.sessions import EngineFailure, atomic_json

from .adapters import HERE, create_session
from .catalog import CONTAINMENT_RULES, ENGINES, HEADLINE_RULES, NATIVE_RULES, SUPPORT
from .common import (
    Session,
    WorkerSession,
    capture,
    eligibility,
    normalize,
    sample_rounds,
)
from .plot import publish, summary
from .probes import additional_cases

ERRORS = (EngineFailure, OSError, ValueError, RuntimeError, psycopg.Error)


def discover(engines: list[str]) -> dict[str, dict[str, Any]]:
    """Check dependencies without launching user applications or acquiring licenses."""
    result: dict[str, dict[str, Any]] = {}
    for engine in ("rust", *engines):
        reason = None
        executable = {
            "rust": os.environ.get(
                "GIS_RUST_BINARY", str(ROOT / "target/release/examples/qgis_benchmark")
            ),
            "qgis": os.environ.get("GIS_QGIS_BINARY", "qgis"),
            "grass": os.environ.get("GIS_GRASS_BINARY", "grass"),
        }.get(engine)
        if executable and not shutil.which(executable):
            reason = f"Executable unavailable: {Path(executable).name}"
        if (
            engine == "gvsig"
            and not Path(os.environ.get("GIS_GVSIG_HOME", "/opt/gvsig")).is_dir()
        ):
            reason = "gvSIG distribution unavailable"
        result[engine] = {"status": "unavailable" if reason else "ready"}
        if reason:
            result[engine]["reason"] = reason
    return result


def metadata(args: argparse.Namespace) -> dict[str, Any]:
    """Capture CPU identity, exact source and tool hashes, versions and run limits."""
    paths = sorted(HERE.glob("*")) + [
        ROOT / "Cargo.lock",
        ROOT / "crates/io/examples/qgis_benchmark.rs",
    ]
    tooling = {
        str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in paths
        if p.is_file()
    }
    sources = sorted(
        p
        for base in (ROOT / "crates", ROOT / "python")
        for p in base.rglob("*")
        if p.suffix in {".rs", ".py", ".toml"}
    )
    source_hash = hashlib.sha256()
    for path in sources:
        source_hash.update(
            str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes()
        )
    revision = os.environ.get("GIS_SOURCE_REVISION")
    if revision is None:
        revision = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip()
    native = (
        platform.system() == "Linux"
        and platform.machine() == "x86_64"
        and os.environ.get("GIS_NATIVE_CI") == "1"
    )
    affinity = vars(os).get("sched_getaffinity")
    return {
        "started_at": datetime.now(UTC).isoformat(),
        "source_revision": revision,
        "source_tree_sha256": source_hash.hexdigest(),
        "tooling_sha256": tooling,
        "platform": platform.platform(),
        "machine": platform.machine(),
        "execution": {
            "native_linux_ci": native,
            "runner": os.environ.get("RUNNER_NAME"),
            "run_url": os.environ.get("GIS_RUN_URL"),
            "cpu_affinity": sorted(affinity(0)) if affinity else None,
        },
        "cpu": Path("/proc/cpuinfo").read_text()
        if Path("/proc/cpuinfo").exists()
        else platform.processor(),
        "memory": Path("/proc/meminfo").read_text()
        if Path("/proc/meminfo").exists()
        else None,
        "packages": Path("/opt/gis-packages.lock").read_text()
        if Path("/opt/gis-packages.lock").exists()
        else None,
        "container_image": os.environ.get("GIS_CONTAINER_IMAGE"),
        "rust_binary_sha256": hashlib.sha256(args.rust.read_bytes()).hexdigest()
        if args.rust.is_file()
        else None,
        "sizes": args.sizes,
        "repetitions": args.repetitions,
        "warmups": args.warmups,
        "timeout_s": args.timeout,
        "startup_timeout_s": args.startup_timeout,
        "crs": "EPSG:3857",
        "tolerance": 0,
        "rust_threads": 1,
        "headline_geometry_scope": "Single points, lines and polygons, including bounded holes and islands",
        "scope": {
            "included": "fresh indexes or topology, rule evaluation, materialized violation geometries",
            "excluded": "application startup, source loading, fresh copies, response extraction, serialization",
            "grass": "native module launch and native vector output writes included",
            "postgis": "server-side SQL workflows including GiST indexes and ANALYZE",
            "qgis": "C++ containment plugin or native indexed geometry workflow with error geometries",
        },
    }


def start(
    engine: str,
    rule: str,
    fixture: dict[str, Any],
    directory: Path,
    args: argparse.Namespace,
    data: dict[str, Any],
) -> Session:
    """Launch a session and record exact engine versions."""
    available = data["engines"][engine].get("available_rules")
    if (
        engine == "gvsig"
        and rule not in NATIVE_RULES
        and available
        and SUPPORT[engine][rule] not in available
    ):
        raise EngineFailure("unsupported", "Mapped rule absent from installed gvSIG")
    session = create_session(engine, rule, fixture, directory, args.startup_timeout)
    data["engines"][engine].update(session.metadata)
    data["engines"][engine]["status"] = "ready"
    return session


def probe_cases(rule: str) -> list[dict[str, Any]]:
    """Keep historical probes and add hole cases to shared containment checks."""
    fixture = FIXTURES[rule]
    cases = [{"name": name, **fixture[name]} for name in ("clean", "violation")]
    cases.extend(case for case in CATALOG["edge_cases"] if case["rule"] == rule)
    hole = {
        "type": "Polygon",
        "coordinates": [
            [[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]],
            [[4, 4], [4, 6], [6, 6], [6, 4], [4, 4]],
        ],
    }
    cases.extend(additional_cases(rule))
    if rule in CONTAINMENT_RULES:
        for name, xy in (("point in hole", [5, 5]), ("point on hole boundary", [4, 5])):
            point = {"type": "Point", "coordinates": xy}
            primary, secondary = (
                ([point], [hole]) if rule.startswith("point/") else ([hole], [point])
            )
            cases.append(
                {"name": name, "input": primary, "other": secondary, "expected": 1}
            )
    return cases


def run_probes(
    rule: str, directory: Path, args: argparse.Namespace, data: dict[str, Any]
) -> tuple[list[dict[str, Any]], dict[str, bool]]:
    """Check each competitor independently against Rust expectations and identities."""
    with ExitStack() as stack:
        gates = {engine: True for engine in args.engines}
        records = []
        resident: dict[str, Session] = {}
        for index, case in enumerate(probe_cases(rule)):
            fixture = write_case(
                directory / f"probe-{index}", case["input"], case.get("other")
            )
            record: dict[str, Any] = {
                "name": case["name"],
                "fixture": fixture,
                "engines": {},
            }
            raw: dict[str, dict[str, Any]] = {}
            for engine in ("rust", *args.engines):
                if engine != "rust" and rule not in SUPPORT[engine]:
                    record["engines"][engine] = {"status": "unsupported"}
                    gates[engine] = False
                    continue
                session = None
                try:
                    if data["engines"][engine]["status"] == "unavailable":
                        raise FileNotFoundError(data["engines"][engine]["reason"])
                    session = resident.get(engine)
                    if session is not None and isinstance(session, WorkerSession):
                        session.reload(fixture, args.startup_timeout)
                    else:
                        session = start(
                            engine,
                            rule,
                            fixture,
                            directory / f"probe-{index}" / engine,
                            args,
                            data,
                        )
                        if engine == "gvsig":
                            resident[engine] = session
                            stack.callback(session.close)
                    response = capture(session, rule, args.timeout)
                except ERRORS as error:
                    response = {"status": "startup_failed", "error": str(error)}
                finally:
                    if session is not None and engine != "gvsig":
                        session.close()
                if engine == "gvsig" and response.get("status") != "ok":
                    failed_session = resident.pop(engine, None)
                    if failed_session is not None:
                        failed_session.close()
                raw[engine] = response
                record["engines"][engine] = response
            rust = raw["rust"]
            expected = False
            if rust["status"] == "ok":
                count = normalize(rule, rust)["count"]
                expected = (
                    count == case["expected"]
                    if "expected" in case
                    else (count == 0 if case["name"] == "clean" else count > 0)
                )
            if expected and case.get("identity"):
                identity = normalize(rule, rust)
                expected = all(
                    identity.get(key) == value
                    for key, value in case["identity"].items()
                )
            if expected and "expected_gaps" in case:
                expected = normalize(rule, rust) == normalize(
                    rule,
                    {
                        "errors": [
                            {"kind": "gap", "geometry": geometry}
                            for geometry in case["expected_gaps"]
                        ]
                    },
                )
            record["rust_expected"] = expected
            for engine in args.engines:
                response = raw.get(engine, {})
                agrees = (
                    expected
                    and response.get("status") == "ok"
                    and normalize(rule, rust) == normalize(rule, response)
                )
                gates[engine] &= bool(agrees)
                record["engines"][engine]["agrees"] = bool(agrees)
                if response.get("status") == "ok" and not agrees:
                    record["engines"][engine]["reason"] = "provenance or count differs"
            records.append(record)
        return records, gates


def measure(
    rule: str,
    fixture: dict[str, Any],
    directory: Path,
    args: argparse.Namespace,
    data: dict[str, Any],
    gates: dict[str, bool],
) -> dict[str, Any]:
    """Sample a fixed engine set and retain all unsupported or failed comparisons."""
    sessions: dict[str, Session] = {}
    failures = {}
    for engine in ("rust", *args.engines):
        if engine != "rust" and rule not in SUPPORT[engine]:
            failures[engine] = {"status": "unsupported", "reason": "unsupported"}
            continue
        try:
            if data["engines"][engine]["status"] == "unavailable":
                raise FileNotFoundError(data["engines"][engine]["reason"])
            sessions[engine] = start(
                engine, rule, fixture, directory / engine, args, data
            )
        except ERRORS as error:
            failures[engine] = {"status": "startup_failed", "reason": str(error)}
    try:
        work = (
            sample_rounds(rule, sessions, args.repetitions, args.warmups, args.timeout)
            if sessions
            else {
                "samples": {},
                "warmups": [],
                "workload_agrees": {},
                "failures": {},
            }
        )
    finally:
        for session in sessions.values():
            session.close()
    work["failures"].update(failures)
    work["comparisons"] = {}
    for engine in args.engines:
        if engine in failures:
            eligible, reason = False, failures[engine]["reason"]
        else:
            baseline_complete = len(
                work["samples"].get("rust", [])
            ) == args.repetitions and all(
                s["status"] == "ok" for s in work["samples"].get("rust", [])
            )
            eligible, reason = eligibility(
                gates[engine] and baseline_complete,
                work["workload_agrees"].get(engine, False),
                work["samples"].get(engine, []),
                args.repetitions,
            )
        work["comparisons"][engine] = {"eligible": eligible, "reason": reason}
    return work


def parse_args() -> argparse.Namespace:
    """Parse workload selection and reproduction settings."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engines", nargs="+", choices=ENGINES, default=list(ENGINES))
    parser.add_argument(
        "--rules",
        nargs="+",
        choices=list(FIXTURES),
        default=[
            *HEADLINE_RULES,
            *(rule for rule in FIXTURES if rule not in HEADLINE_RULES),
        ],
    )
    parser.add_argument("--sizes", nargs="+", type=int, default=[1000, 10000, 100000])
    parser.add_argument("--repetitions", type=int, default=10)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--startup-timeout", type=float, default=180)
    parser.add_argument("--output", type=Path, default=HERE / "results")
    parser.add_argument("--work", type=Path, default=ROOT / "builds/gis")
    parser.add_argument("--smoke", action="store_true")
    args = parser.parse_args()
    if (
        min(args.sizes) < 1
        or args.repetitions < 1
        or args.warmups < 0
        or min(args.timeout, args.startup_timeout) <= 0
    ):
        parser.error(
            "sizes, repetitions and deadlines must be positive; warmups nonnegative"
        )
    if len(set(args.engines)) != len(args.engines) or len(set(args.rules)) != len(
        args.rules
    ):
        parser.error("engines and rules must be unique")
    if args.smoke:
        args.rules, args.sizes, args.repetitions, args.warmups = (
            list(HEADLINE_RULES),
            [10],
            1,
            0,
        )
    args.rust = Path(
        os.environ.get(
            "GIS_RUST_BINARY", str(ROOT / "target/release/examples/qgis_benchmark")
        )
    )
    args.work = args.work / datetime.now(UTC).strftime("%Y%m%dT%H%M%S%f")
    return args


def main() -> None:
    """Save measurements after each workload and export the final auditable report."""
    args = parse_args()
    os.environ["RAYON_NUM_THREADS"] = "1"
    args.output.mkdir(parents=True, exist_ok=True)
    data: dict[str, Any] = {
        "schema_version": 3,
        "metadata": metadata(args),
        "engines": discover(args.engines),
        "rules": [],
    }
    for rule in args.rules:
        print(f"Probes: {rule}", flush=True)
        directory = args.work / rule.replace("/", "_")
        probes, gates = run_probes(rule, directory, args, data)
        record = {
            "rule": rule,
            "operations": {
                "rust": "Topology.validate with fresh caches",
                **{
                    engine: SUPPORT[engine].get(rule, "unsupported")
                    for engine in args.engines
                },
            },
            "probes": probes,
            "workloads": [],
        }
        data["rules"].append(record)
        for size in args.sizes:
            print(f"Measure: {rule}, {size:,} features", flush=True)
            primary, secondary, invalid_blocks = workload(rule, size)
            target = directory / str(size)
            fixture = write_case(target, primary, secondary)
            del primary, secondary
            record["workloads"].append(
                {
                    "size": size,
                    "fixture": fixture,
                    "invalid_blocks": invalid_blocks,
                    **measure(rule, fixture, target, args, data, gates),
                }
            )
            atomic_json(args.output / "results.json", data)
            work = record["workloads"][-1]
            for engine, comparison in work["comparisons"].items():
                stats = summary(work["samples"].get(engine, []))
                duration = (
                    f"{stats['median_s'] * 1000:.3f} ms" if stats else "no timings"
                )
                statuses = sorted(
                    {s["status"] for s in work["samples"].get(engine, [])}
                )
                print(
                    f"{engine}: {comparison['reason']}; {duration}; statuses={statuses}",
                    flush=True,
                )
    data["metadata"]["finished_at"] = datetime.now(UTC).isoformat()
    atomic_json(args.output / "results.json", data)
    publish(data, args.output)
    if args.smoke:
        failed = [
            f"{r['rule']}: {name}"
            for r in data["rules"]
            for name in args.engines
            if name in ENGINES
            if not r["workloads"][0]["comparisons"].get(name, {}).get("eligible")
        ]
        if failed:
            raise SystemExit("Smoke checks failed: " + ", ".join(failed))


if __name__ == "__main__":
    main()
