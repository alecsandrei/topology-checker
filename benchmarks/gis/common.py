"""Engine-independent correctness gates, scheduling and process IPC."""

import hashlib
import json
import os
import shutil
import signal
import subprocess
from collections.abc import Mapping
from pathlib import Path
from typing import Any, Protocol

import psycopg
from shapely import from_wkb, from_wkt, to_wkb
from shapely import normalize as canonical
from shapely.errors import GEOSException
from shapely.geometry import MultiPolygon, Polygon, shape
from shapely.ops import unary_union

from benchmarks.qgis.run import EngineFailure, atomic_json, close_process, wait_file


class Session(Protocol):
    """One loaded workload with fresh validation state for every sample."""

    metadata: dict[str, Any]

    def sample(self, rule: str, timeout: float) -> dict[str, Any]:
        """Return timing and violations after validation completes."""
        ...

    def close(self) -> None:
        """Release only benchmark-owned resources."""
        ...


def normalize(rule: str, result: dict[str, Any]) -> dict[str, Any]:
    """Compare pairs, affected IDs and topologically canonical bounded voids."""
    errors = [e for e in result["errors"] if e["kind"] != "outer-boundary"]
    ids = sorted(
        {e["id"] for e in errors if e.get("id") is not None and e.get("layer", 0) == 0}
    )
    count = len(errors)
    identity: dict[str, Any] = {}
    if rule in {"point/must-not-overlap", "polygon/must-not-overlap"}:
        pairs = sorted({tuple(sorted((e["id"], e["destination"]))) for e in errors})
        identity["pairs"] = [list(pair) for pair in pairs]
        count = len(pairs)
        ids = sorted({pair[0] for pair in pairs})
    if rule in {
        "line/must-not-have-dangles",
        "line/must-not-have-pseudo-nodes",
        "line/endpoints-must-be-covered-by-points",
    }:
        count = len(ids)
    if rule == "polygon/must-not-have-gaps":
        ids = []

        def surface(error: dict[str, Any]) -> Any:
            if "geometry_wkb" in error:
                geometry = from_wkb(bytes.fromhex(error["geometry_wkb"]))
            else:
                value = error["geometry"]
                geometry = from_wkt(value) if isinstance(value, str) else shape(value)
            return (
                Polygon(geometry.coords)
                if geometry.geom_type == "LineString"
                else geometry
            )

        gaps = unary_union([surface(e) for e in errors])
        # Rust reports union rings. Interior exterior rings are covered islands.
        islands = [
            surface(e)
            for e in result["errors"]
            if e["kind"] == "outer-boundary" and gaps.covers(surface(e))
        ]
        if islands:
            gaps = gaps.difference(unary_union(islands))
        geometry = canonical(gaps.simplify(0))
        count = (
            0
            if geometry.is_empty
            else (len(geometry.geoms) if isinstance(geometry, MultiPolygon) else 1)
        )
        identity["gap_geometry_sha256"] = hashlib.sha256(
            to_wkb(geometry, byte_order=1)
        ).hexdigest()
    return {"count": count, "primary_ids": ids, **identity}


def fingerprint(rule: str, response: dict[str, Any]) -> dict[str, Any]:
    """Keep raw counts and exact normalized identity without large result arrays."""
    normalized = normalize(rule, response)
    return {
        "status": "ok",
        "seconds": response["seconds"],
        "raw_count": response["raw_count"],
        "normalized_count": normalized["count"],
        "result_sha256": hashlib.sha256(
            json.dumps(normalized, sort_keys=True).encode()
        ).hexdigest(),
        "primary_ids_sha256": hashlib.sha256(
            json.dumps(normalized["primary_ids"]).encode()
        ).hexdigest(),
    }


def eligibility(
    probes_passed: bool,
    workload_agrees: bool,
    samples: list[dict[str, Any]],
    repetitions: int,
) -> tuple[bool, str]:
    """Require complete measurements and correct probes before pairing."""
    if not probes_passed:
        return False, "probes differ or failed"
    if not workload_agrees:
        return False, "workload differs"
    if len(samples) != repetitions or any(s["status"] != "ok" for s in samples):
        return False, "incomplete measurements"
    return True, "paired"


def capture(session: Session, rule: str, timeout: float) -> dict[str, Any]:
    """Keep process failures distinct from real measurements."""
    try:
        response = session.sample(rule, timeout)
        if response["status"] == "ok":
            seconds = response["seconds"]
            if not isinstance(seconds, (int, float)) or not 0 < seconds < float("inf"):
                raise ValueError("invalid validation duration")
            if response["raw_count"] != len(response["errors"]):
                raise ValueError("result count does not match materialized violations")
            normalize(rule, response)
        return response
    except (
        EngineFailure,
        BrokenPipeError,
        OSError,
        ValueError,
        TypeError,
        GEOSException,
        KeyError,
        RuntimeError,
        psycopg.Error,
    ) as error:
        return {"status": getattr(error, "status", "failed"), "error": str(error)}


def sample_rounds(
    rule: str,
    sessions: Mapping[str, Session],
    repetitions: int,
    warmups: int,
    timeout: float,
) -> dict[str, Any]:
    """Rotate a fixed engine order and stop failed engines without retries."""
    names = list(sessions)
    measured: dict[str, Any] = {
        "samples": {name: [] for name in names},
        "warmups": [],
        "workload_agrees": {name: True for name in names if name != "rust"},
        "failures": {},
    }
    failed: set[str] = set()
    for iteration in range(repetitions + warmups):
        offset = iteration % len(names)
        order = names[offset:] + names[:offset]
        raw: dict[str, dict[str, Any]] = {}
        for position, engine in enumerate(order):
            if engine in failed:
                sample = {"status": "not_run", "reason": measured["failures"][engine]}
            else:
                response = capture(sessions[engine], rule, timeout)
                if response["status"] != "ok":
                    failed.add(engine)
                    measured["failures"][engine] = response
                    sessions[engine].close()
                    sample = response
                else:
                    raw[engine] = response
                    sample = fingerprint(rule, response)
            sample.update(iteration=iteration, order=position)
            if iteration < warmups:
                measured["warmups"].append({"engine": engine, **sample})
            else:
                measured["samples"][engine].append(sample)
        for engine in measured["workload_agrees"]:
            if "rust" not in raw or engine not in raw:
                measured["workload_agrees"][engine] = False
            elif normalize(rule, raw["rust"]) != normalize(rule, raw[engine]):
                measured["workload_agrees"][engine] = False
    return measured


class WorkerSession:
    """An embedded interpreter worker using phase-aware file IPC."""

    def __init__(
        self,
        command: list[str],
        directory: Path,
        startup_timeout: float,
        environment: dict[str, str] | None = None,
    ) -> None:
        """Launch an isolated worker and wait for its version response."""
        self.directory = directory
        self.startup_timeout = startup_timeout
        directory.mkdir(parents=True, exist_ok=True)
        self.log = (directory / "engine.log").open("w")
        self.process = subprocess.Popen(
            command,
            stdout=self.log,
            stderr=self.log,
            env=dict(os.environ, **(environment or {})),
            start_new_session=True,
        )
        self.closed = False
        try:
            self.metadata = wait_file(
                directory / "response.json", self.process, startup_timeout
            )
        except (EngineFailure, OSError, ValueError):
            self.close()
            raise

    def request(self, command: dict[str, Any], timeout: float) -> dict[str, Any]:
        """Send a request whose preparation and validation have separate deadlines."""
        (self.directory / "phase.txt").unlink(missing_ok=True)
        atomic_json(self.directory / "request.json", command)
        return wait_file(
            self.directory / "response.json",
            self.process,
            self.startup_timeout,
            timeout,
        )

    def sample(self, rule: str, timeout: float) -> dict[str, Any]:
        """Validate once, leaving response extraction outside the clock."""
        return self.request({"action": "sample", "rule": rule}, timeout)

    def reload(self, fixture: dict[str, Any], timeout: float) -> None:
        """Load another fixture in a resident gvSIG process, outside validation."""
        from benchmarks.qgis.fixtures import ROOT

        self.request(
            {
                "action": "load",
                "fixture": {
                    name: str(ROOT / data["path"]) for name, data in fixture.items()
                },
            },
            timeout,
        )

    def close(self) -> None:
        """Stop the worker and its child processes without touching user processes."""
        if self.closed:
            return
        self.closed = True
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
        close_process(self.process)
        # A launcher can exit while its JVM is still flushing preferences. Stop
        # every remaining process in this worker's private group before cleanup.
        try:
            os.killpg(self.process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.log.close()
        app = self.directory / "app"
        native_log = app / "preferences/gvSIG/gvSIG.log"
        if native_log.is_file():
            with native_log.open("rb") as stream:
                stream.seek(max(0, native_log.stat().st_size - 256000))
                (self.directory / "gvsig.log").write_bytes(stream.read())
        if app.is_dir():
            shutil.rmtree(app)


def command_output(parts: list[str]) -> str:
    """Collect bounded diagnostic commands without leaking environment variables."""
    return subprocess.check_output(parts, text=True, timeout=30).strip()
