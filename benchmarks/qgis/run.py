"""Run preloaded, sequential Rust and installed QGIS Topology Checker samples."""

import argparse
import hashlib
import json
import os
import platform
import select
import subprocess
import time
from collections import Counter
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

if __package__:
    from .fixtures import CATALOG, FIXTURES, HERE, MAPPINGS, ROOT, workload, write_case
else:
    from fixtures import CATALOG, FIXTURES, HERE, MAPPINGS, ROOT, workload, write_case

DEFAULT_QGIS = "/Applications/QGIS-final-4_2_2.app/Contents/MacOS/QGIS-final-4_2_2"


class EngineFailure(RuntimeError):
    """An engine failed, timed out or returned an invalid response."""

    def __init__(self, status: str, message: str) -> None:
        """Retain a machine-readable failure category."""
        super().__init__(message)
        self.status = status


def atomic_json(path: Path, value: Any) -> None:
    """Write JSON atomically so an interrupted run retains previous results."""
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")
    temporary.replace(path)


def wait_file(
    path: Path,
    process: subprocess.Popen,
    timeout: float,
    validation_timeout: float | None = None,
) -> dict:
    """Wait for an atomic response, observing process exit and a deadline."""
    deadline = time.monotonic() + timeout
    current_phase = "preparing"
    while not path.exists():
        marker = path.parent / "phase.txt"
        if validation_timeout is not None and marker.exists():
            new_phase = marker.read_text()
            if new_phase != current_phase:
                current_phase = new_phase
                allowance = validation_timeout if new_phase == "validating" else timeout
                deadline = time.monotonic() + allowance
        if process.poll() is not None:
            raise EngineFailure("failed", f"Engine exited: {process.returncode}")
        if time.monotonic() >= deadline:
            raise EngineFailure("timeout", f"Deadline exceeded during {current_phase}")
        time.sleep(0.02)
    response = json.loads(path.read_text())
    path.unlink()
    if response["status"] == "failed":
        raise EngineFailure("failed", response["error"])
    return response


def close_process(process: subprocess.Popen) -> None:
    """Stop only a benchmark-owned process and reap it."""
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    for stream in (process.stdin, process.stdout, process.stderr):
        if stream is not None:
            stream.close()


class QgisSession:
    """One isolated QGIS profile, with one rule configured per sample."""

    def __init__(
        self, executable: str, directory: Path, startup_timeout: float
    ) -> None:
        """Start the actual GUI binary, leaving regular profiles untouched."""
        self.directory = directory
        self.startup_timeout = startup_timeout
        directory.mkdir(parents=True, exist_ok=True)
        profile = directory / "profiles" / "profiles" / "benchmark" / "qgis.org"
        profile.mkdir(parents=True, exist_ok=True)
        settings = "[Plugins]\nlibplugin_topology=true\n[authentication]\nuse-password-helper=false\ngenerate-random-password-for-keychain=false\n[locale]\noverrideFlag=true\nuserLocale=en_US\n"
        (profile / "QGIS4.ini").write_text(settings)
        (profile / "QGIS3.ini").write_text(settings)
        global_settings = directory / "settings.ini"
        global_settings.write_text(settings)
        self.log = (directory / "qgis.log").open("w")
        environment = dict(
            os.environ,
            TOPOLOGY_QGIS_SESSION=str(directory.resolve()),
            QGIS_CONTINUOUS_INTEGRATION_RUN="true",
            QT_QPA_PLATFORM=os.environ.get("QT_QPA_PLATFORM", "offscreen"),
        )
        # QGIS embeds system Python. A runner's venv can otherwise redirect its
        # interpreter prefix and hide the distribution's Qt bindings.
        virtual_env = environment.pop("VIRTUAL_ENV", None)
        environment.pop("PYTHONHOME", None)
        if virtual_env:
            environment["PATH"] = os.pathsep.join(
                part
                for part in environment.get("PATH", "").split(os.pathsep)
                if Path(part) != Path(virtual_env) / "bin"
            )
        # All IPC files are unique to this session, never to the user's app.
        self.process = subprocess.Popen(
            [
                executable,
                "--nologo",
                "--noversioncheck",
                "--hide-browser",
                "--profiles-path",
                str((directory / "profiles").resolve()),
                "--profile",
                "benchmark",
                "--globalsettingsfile",
                str(global_settings.resolve()),
                "--code",
                str(HERE / "qgis_startup.py"),
            ],
            env=environment,
            stdout=self.log,
            stderr=self.log,
        )
        try:
            self.metadata = wait_file(
                directory / "response.json", self.process, startup_timeout
            )
        except (EngineFailure, ValueError, OSError):
            self.close()
            raise

    def command(self, command: dict, timeout: float) -> dict:
        """Submit one command and wait until its validation has completed."""
        atomic_json(self.directory / "request.json", command)
        return wait_file(self.directory / "response.json", self.process, timeout)

    def load(self, fixture: dict, timeout: float) -> None:
        """Preload geometries before any sample requests."""
        self.command(
            {
                "action": "load",
                "input": str(ROOT / fixture["input"]["path"]),
                "other": (
                    str(ROOT / fixture["other"]["path"]) if "other" in fixture else None
                ),
            },
            timeout,
        )

    def sample(self, rule: str, timeout: float) -> dict:
        """Time the plugin's Validate All slot, including rows and markers."""
        (self.directory / "phase.txt").unlink(missing_ok=True)
        atomic_json(
            self.directory / "request.json",
            {"action": "sample", "rule": MAPPINGS[rule][0]},
        )
        return wait_file(
            self.directory / "response.json",
            self.process,
            self.startup_timeout,
            timeout,
        )

    def close(self) -> None:
        """Release the isolated app and its log handle."""
        close_process(self.process)
        self.log.close()


class RustSession:
    """A production importer followed by fresh layers for each request."""

    def __init__(
        self,
        executable: Path,
        rule: str,
        fixture: dict,
        directory: Path,
        startup_timeout: float,
    ) -> None:
        """Preload one fixture with Rayon constrained to a single thread."""
        self.log = (directory / "rust.log").open("w")
        self.startup_timeout = startup_timeout
        self.buffer = b""
        command = [str(executable), rule, str(ROOT / fixture["input"]["path"])]
        if "other" in fixture:
            command.append(str(ROOT / fixture["other"]["path"]))
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.log,
            bufsize=0,
            env=dict(os.environ, RAYON_NUM_THREADS="1"),
        )
        try:
            self.metadata = self.read(startup_timeout)
        except (EngineFailure, ValueError, OSError):
            self.close()
            raise

    def read(self, timeout: float) -> dict:
        """Read exactly one newline response with an external deadline."""
        assert self.process.stdout is not None
        deadline = time.monotonic() + timeout
        while b"\n" not in self.buffer:
            remaining = max(0, deadline - time.monotonic())
            if not select.select([self.process.stdout], [], [], remaining)[0]:
                raise EngineFailure("timeout", f"No Rust response within {timeout:g}s")
            chunk = os.read(self.process.stdout.fileno(), 65536)
            if not chunk:
                raise EngineFailure("failed", f"Rust exited: {self.process.poll()}")
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b"\n", 1)
        return json.loads(line)

    def sample(self, rule: str, timeout: float) -> dict:
        """Request one validation; IPC and serialization are outside its clock."""
        assert self.process.stdin is not None
        self.process.stdin.write(b"sample\n")
        self.process.stdin.flush()
        preparing = self.read(self.startup_timeout)
        if preparing["status"] != "validating":
            raise EngineFailure("failed", "Expected Rust validation phase")
        finished = self.read(timeout)
        if finished["status"] != "extracting":
            raise EngineFailure("failed", "Expected Rust extraction phase")
        return self.read(self.startup_timeout)

    def close(self) -> None:
        """Release the benchmark process and log handle."""
        close_process(self.process)
        self.log.close()


def normalized(rule: str, result: dict, engine: str) -> dict:
    """Compare visible primary IDs, collapsing endpoint rows and ignoring exteriors."""
    errors = [e for e in result["errors"] if e["kind"] != "outer-boundary"]
    ids = sorted(
        {
            e["id"]
            for e in errors
            if e["id"] is not None and (engine == "qgis" or e.get("layer") == 0)
        }
    )
    count = len(errors)
    if rule in {
        "line/must-not-have-dangles",
        "line/must-not-have-pseudo-nodes",
        "line/endpoints-must-be-covered-by-points",
    }:
        count = len(ids)
    if rule == "polygon/must-not-have-gaps":
        ids = []  # Gap records do not expose comparable feature provenance.
    return {"count": count, "primary_ids": ids}


def compact(rule: str, result: dict, engine: str) -> dict:
    """Retain timing, raw counts, type counts and normalized ID fingerprints."""
    norm = normalized(rule, result, engine)
    return {
        "status": result["status"],
        "seconds": result["seconds"],
        "raw_count": result["raw_count"],
        "types": dict(Counter(e["kind"] for e in result["errors"])),
        "normalized_count": norm["count"],
        "primary_id_count": len(norm["primary_ids"]),
        "primary_ids_sha256": hashlib.sha256(
            json.dumps(norm["primary_ids"]).encode()
        ).hexdigest(),
    }


def compare(rule: str, rust: dict, qgis: dict) -> bool:
    """Require equal normalized counts and all visible primary feature IDs."""
    return normalized(rule, rust, "rust") == normalized(rule, qgis, "qgis")


def capture(session: Any, rule: str, timeout: float) -> dict:
    """Turn engine errors into explicit records without disguising failures."""
    try:
        return session.sample(rule, timeout)
    except (EngineFailure, BrokenPipeError, OSError, ValueError) as error:
        return {
            "status": error.status if isinstance(error, EngineFailure) else "failed",
            "error": str(error),
        }


def measure(
    rule: str,
    fixture: dict,
    args: argparse.Namespace,
    directory: Path,
    qgis: QgisSession | None,
    count: int,
    warmups: int,
) -> dict:
    """Alternate sequential engines and never retry a failed sample implicitly."""
    result: dict[str, Any] = {
        "rust": [],
        "qgis": [],
        "warmups": [],
        "workload_agrees": True,
    }
    sessions: dict[str, Any] = {}
    failures: dict[str, dict] = {}
    if qgis is None and rule in MAPPINGS:
        failures["qgis"] = {
            "status": "unavailable",
            "error": "QGIS session failed to start or stopped after a failed sample",
        }
    try:
        try:
            sessions["rust"] = RustSession(
                args.rust, rule, fixture, directory, args.startup_timeout
            )
            result["rust_metadata"] = sessions["rust"].metadata
        except (EngineFailure, OSError, ValueError) as error:
            failures["rust"] = {"status": "startup_failed", "error": str(error)}
        if qgis is not None:
            try:
                qgis.load(fixture, args.startup_timeout)
                sessions["qgis"] = qgis
            except (EngineFailure, OSError, ValueError) as error:
                failures["qgis"] = {"status": "startup_failed", "error": str(error)}
        for index in range(count + warmups):
            samples = {}
            order = ["rust", "qgis"] if index % 2 == 0 else ["qgis", "rust"]
            for engine in order:
                if engine not in sessions and engine not in failures:
                    continue
                if engine in failures:
                    sample = {"status": "not_run", "reason": failures[engine]}
                else:
                    raw = capture(sessions[engine], rule, args.timeout)
                    if raw["status"] != "ok":
                        failures[engine] = raw
                        sample = raw
                        sessions[engine].close()
                        del sessions[engine]
                    else:
                        samples[engine] = raw
                        sample = compact(rule, raw, engine)
                sample["order"] = order.index(engine)
                sample["iteration"] = index
                if index < warmups:
                    result["warmups"].append({"engine": engine, **sample})
                else:
                    result[engine].append(sample)
            if len(samples) == 2 and not compare(
                rule, samples["rust"], samples["qgis"]
            ):
                result["workload_agrees"] = False
        result["failures"] = failures
    finally:
        if "rust" in sessions:
            sessions["rust"].close()
    return result


def probes(
    rule: str, args: argparse.Namespace, directory: Path, qgis: QgisSession | None
) -> list[dict]:
    """Run clean, deliberately invalid, and boundary probes before performance."""
    fixture = FIXTURES[rule]
    cases = [{"name": name, **fixture[name]} for name in ("clean", "violation")]
    cases += [
        {
            "name": c["name"],
            "input": c["input"],
            "other": c.get("other"),
            "expected": c["expected"],
        }
        for c in CATALOG["edge_cases"]
        if c["rule"] == rule
    ]
    output = []
    for index, case in enumerate(cases):
        case_directory = directory / f"probe-{index}"
        files = write_case(case_directory, case["input"], case["other"])
        item: dict[str, Any] = {"name": case["name"], "fixture": files}
        rust = None
        try:
            rust = RustSession(
                args.rust, rule, files, case_directory, args.startup_timeout
            )
            item["rust"] = capture(rust, rule, args.timeout)
        except (EngineFailure, OSError, ValueError) as error:
            item["rust"] = {"status": "startup_failed", "error": str(error)}
        finally:
            if rust is not None:
                rust.close()
        if item["rust"]["status"] != "ok":
            item["rust_expected"] = False
        else:
            value = normalized(rule, item["rust"], "rust")["count"]
            item["rust_expected"] = (
                value == case["expected"]
                if "expected" in case
                else (value == 0 if case["name"] == "clean" else value > 0)
            )
        if qgis is not None and qgis.process.poll() is None:
            try:
                qgis.load(files, args.startup_timeout)
                item["qgis"] = capture(qgis, rule, args.timeout)
            except (EngineFailure, OSError, ValueError) as error:
                item["qgis"] = {"status": "startup_failed", "error": str(error)}
            if item["qgis"]["status"] != "ok":
                qgis.close()
            item["agrees"] = item["rust"]["status"] == item["qgis"][
                "status"
            ] == "ok" and compare(rule, item["rust"], item["qgis"])
        else:
            item["agrees"] = None
        output.append(item)
    return output


def metadata(args: argparse.Namespace) -> dict:
    """Record exact versions, hardware, source revision and executable hashes."""

    def command(*parts: str) -> str:
        return subprocess.check_output(parts, cwd=ROOT, text=True).strip()

    hardware = {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "logical_cpus": os.cpu_count(),
    }
    if platform.system() == "Darwin":
        for key in (
            "hw.model",
            "hw.memsize",
            "hw.physicalcpu",
            "machdep.cpu.brand_string",
        ):
            hardware[key] = command("sysctl", "-n", key)
    executable = Path(args.qgis).resolve()
    return {
        "started_at": datetime.now(UTC).isoformat(),
        "git_revision": command("git", "rev-parse", "HEAD"),
        "git_dirty": command("git", "status", "--porcelain"),
        "rustc": command("rustc", "-Vv"),
        "cargo": command("cargo", "-V"),
        "python": platform.python_version(),
        "uv": command("uv", "--version"),
        "qgis_executable": str(executable),
        "qgis_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
        "rust_binary_sha256": hashlib.sha256(args.rust.read_bytes()).hexdigest(),
        "tooling_sha256": {
            p.name: hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(HERE.glob("*.py"))
            + [
                HERE / "fixtures.json",
                ROOT / "crates/io/examples/qgis_benchmark.rs",
                HERE / "uv.lock",
                HERE / "pyproject.toml",
                ROOT / "Cargo.lock",
            ]
        },
        "hardware": hardware,
        "sizes": args.sizes,
        "warmups": args.warmups,
        "repetitions": args.repetitions,
        "validation_timeout_s": args.timeout,
        "startup_timeout_s": args.startup_timeout,
        "rust_threads": 1,
        "crs": "EPSG:3857",
        "tolerance": 0,
        "scope": {
            "rust": "rule registration plus validation on fresh layers of preloaded geometries, includes cleaning and index construction",
            "qgis": "C++ Topology Checker Validate All on fresh memory layers, includes feature traversal, indexes, result rows and error markers; canvas frozen, widget painting disabled",
            "excluded": "application startup, file loading, fresh-layer copying, IPC, result extraction, report serialization, map painting",
        },
    }


def parse_args() -> argparse.Namespace:
    """Parse a single reproducible orchestration command."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qgis", default=DEFAULT_QGIS)
    parser.add_argument("--sizes", nargs="+", type=int, default=[1000, 10000, 100000])
    parser.add_argument("--repetitions", type=int, default=10)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--startup-timeout", type=float, default=180)
    parser.add_argument("--output", type=Path, default=HERE / "results")
    parser.add_argument("--work", type=Path, default=ROOT / "builds/qgis")
    parser.add_argument(
        "--rules", nargs="+", choices=list(FIXTURES), default=list(FIXTURES)
    )
    parser.add_argument("--skip-build", action="store_true")
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
    if not args.skip_build:
        subprocess.run(
            [
                "cargo",
                "build",
                "-p",
                "topology-checker-io",
                "--features",
                "gdal",
                "--release",
                "--example",
                "qgis_benchmark",
                "--locked",
            ],
            cwd=ROOT,
            check=True,
        )
    args.rust = ROOT / "target/release/examples/qgis_benchmark"
    catalog = json.loads(
        subprocess.check_output([str(args.rust), "catalog"], text=True)
    )
    if set(catalog) != set(FIXTURES) or len(catalog) != 38:
        parser.error("fixture coverage must match all 38 core rules")
    if not Path(args.qgis).is_file():
        parser.error("QGIS executable not found; specify --qgis")
    args.output.mkdir(parents=True, exist_ok=True)
    args.work = args.work / datetime.now(UTC).strftime("%Y%m%dT%H%M%S%f")
    args.work.mkdir(parents=True)
    return args


def main() -> None:
    """Run all probes and workloads, saving partial results after each workload."""
    args = parse_args()
    result: dict[str, Any] = {
        "schema_version": 1,
        "metadata": metadata(args),
        "rules": [],
    }
    for rule in args.rules:
        print(f"Probes: {rule}", flush=True)
        directory = args.work / rule.replace("/", "_")
        directory.mkdir()
        qgis = None
        record: dict[str, Any] = {
            "rule": rule,
            "mapping": MAPPINGS.get(rule),
            "workloads": [],
        }
        try:
            if rule in MAPPINGS:
                try:
                    qgis = QgisSession(
                        args.qgis, directory / "session", args.startup_timeout
                    )
                    result["metadata"]["qgis"] = qgis.metadata
                    if MAPPINGS[rule][0] not in qgis.metadata["rules"]:
                        raise EngineFailure(
                            "failed", "Mapped rule absent from installed plugin"
                        )
                except (EngineFailure, OSError, ValueError) as error:
                    record["qgis_failure"] = str(error)
            record["probes"] = probes(rule, args, directory, qgis)
            record["correctness_passed"] = all(
                p["rust_expected"] for p in record["probes"]
            )
            record["paired_gate"] = (
                rule in MAPPINGS
                and record["correctness_passed"]
                and all(p["agrees"] is True for p in record["probes"])
            )
            for size in args.sizes:
                print(f"Measure: {rule}, {size:,} primary features", flush=True)
                primary, secondary, bad_blocks = workload(rule, size)
                case_directory = directory / str(size)
                files = write_case(case_directory, primary, secondary)
                del primary, secondary
                timings = measure(
                    rule,
                    files,
                    args,
                    case_directory,
                    qgis,
                    args.repetitions,
                    args.warmups,
                )
                item = {
                    "size": size,
                    "fixture": files,
                    "invalid_blocks": bad_blocks,
                    **timings,
                }
                item["paired_eligible"] = (
                    record["paired_gate"]
                    and item["workload_agrees"]
                    and not item["failures"]
                    and len(item["qgis"]) == args.repetitions
                )
                record["workloads"].append(item)
                atomic_json(
                    args.output / "results.json",
                    {**result, "rules": result["rules"] + [record]},
                )
                if qgis is not None and qgis.process.poll() is not None:
                    qgis = None
            result["rules"].append(record)
        finally:
            if qgis is not None:
                qgis.close()
    result["metadata"]["finished_at"] = datetime.now(UTC).isoformat()
    atomic_json(args.output / "results.json", result)
    from plot import publish

    publish(args.output / "results.json", args.output)
    print(f"Saved {args.output / 'results.json'}", flush=True)


if __name__ == "__main__":
    main()
