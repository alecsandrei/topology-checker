"""Exercise benchmark process deadlines and relocated source provenance."""

import hashlib
import json
import subprocess
import sys
from argparse import Namespace
from pathlib import Path

import pytest

from benchmarks.gis.fixtures import ROOT, workload, write_case
from benchmarks.gis.plot import quantile
from benchmarks.gis.run import metadata
from benchmarks.gis.sessions import EngineFailure, close_process, wait_file


def test_process_failure_and_deadline(tmp_path: Path) -> None:
    """Timeouts and exited workers produce explicit errors and are reaped."""
    process = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
    try:
        with pytest.raises(EngineFailure, match="Deadline exceeded"):
            wait_file(tmp_path / "missing", process, 0.001)
    finally:
        close_process(process)
    assert process.poll() is not None
    process = subprocess.Popen([sys.executable, "-c", "raise SystemExit(3)"])
    process.wait(timeout=10)
    with pytest.raises(EngineFailure, match="Engine exited"):
        wait_file(tmp_path / "missing", process, 1)


def test_quantile() -> None:
    """Retain the linear interpolation used by the published statistics."""
    assert quantile([4, 1, 3, 2], 0.25) == 1.75
    assert quantile([1, 2, 3, 4], 0.75) == 3.25
    with pytest.raises(ValueError):
        quantile([], 0.5)
    with pytest.raises(ValueError):
        quantile([1], 2)


def test_relocated_source_provenance(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """New runs hash their actual runtime files under the consolidated paths."""
    monkeypatch.setenv("GIS_SOURCE_REVISION", "test")
    args = Namespace(
        rust=tmp_path / "absent-binary",
        sizes=[1000],
        repetitions=10,
        warmups=2,
        timeout=60,
        startup_timeout=180,
    )
    tooling = metadata(args)["tooling_sha256"]
    for name in ("fixtures.py", "fixtures.json", "sessions.py", "qgis_startup.py"):
        path = f"benchmarks/gis/{name}"
        assert tooling[path] == hashlib.sha256((ROOT / path).read_bytes()).hexdigest()
    assert not any(path.startswith("benchmarks/qgis/") for path in tooling)


def test_published_fixtures_survive_relocation(tmp_path: Path) -> None:
    """Reproduce recorded input hashes without changing historical measurements."""
    path = ROOT / "benchmarks/gis/results/results.json"
    if not path.is_file():
        pytest.skip("Published reports are excluded from the runtime container")
    published = json.loads(path.read_text())
    for record in published["rules"]:
        for measured in record["workloads"]:
            primary, secondary, invalid = workload(record["rule"], measured["size"])
            files = write_case(tmp_path, primary, secondary)
            assert invalid == measured["invalid_blocks"]
            for name, fixture in files.items():
                assert fixture["sha256"] == measured["fixture"][name]["sha256"]
