"""Exercise fixture identity, correctness gates, statistics and failure reporting."""

import json
import subprocess
import sys
from argparse import Namespace
from pathlib import Path
from typing import cast

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import FIXTURES, MAPPINGS, collection, translate, workload, write_case  # noqa: E402
from plot import publish, quantile, summary  # noqa: E402
from run import EngineFailure, capture, compare, measure, normalized, wait_file  # noqa: E402


def test_catalog() -> None:
    """Every registered rule has exactly one deterministic fixture."""
    source = Path(__file__).resolve().parents[3] / "crates/core/src/rule.rs"
    import re

    rules = re.findall(r'\([A-Z]\w+, "([\w/-]+)"', source.read_text())
    assert len(FIXTURES) == len(rules) == 38
    assert set(FIXTURES) == set(rules)
    assert set(MAPPINGS) <= set(FIXTURES)
    assert len(MAPPINGS) == 13


@pytest.mark.parametrize("rule", list(FIXTURES))
def test_deterministic(rule: str, tmp_path: Path) -> None:
    """Exact primary sizes, fixed violation rate and hashes survive regeneration."""
    first = workload(rule, 1000)
    assert first == workload(rule, 1000)
    assert len(first[0]) == 1000
    assert first[2] in (5, 10)
    files = write_case(tmp_path, first[0], first[1])
    again = write_case(tmp_path, first[0], first[1])
    assert files == again
    data = json.loads((tmp_path / "input.geojson").read_text())
    assert [f["id"] for f in data["features"]] == list(range(1, 1001))
    assert all(f["id"] == f["properties"]["bench_id"] for f in data["features"])
    assert files["input"]["coordinates"] >= 1000


def test_translation() -> None:
    """Coordinate translation preserves nesting and the source geometry."""
    original = [[[1, 2], [3, 4]]]
    assert translate(original, 10, 20) == [[[11, 22], [13, 24]]]
    assert original == [[[1, 2], [3, 4]]]
    assert collection([], "empty")["features"] == []
    with pytest.raises(ValueError):
        workload(next(iter(FIXTURES)), 0)


def test_normalization() -> None:
    """Endpoint aggregation and outer-boundary filtering retain raw counts."""
    rust = {"errors": [{"kind": "dangle", "id": 1, "layer": 0}], "raw_count": 1}
    qgis = {
        "errors": [{"kind": "dangling", "id": 1}, {"kind": "dangling", "id": 1}],
        "raw_count": 2,
    }
    assert compare("line/must-not-have-dangles", rust, qgis)
    assert qgis["raw_count"] == 2
    assert not compare("point/must-not-overlap", rust, qgis)
    gaps = {
        "errors": [{"kind": "outer-boundary", "id": None}, {"kind": "gap", "id": None}]
    }
    assert normalized("polygon/must-not-have-gaps", gaps, "rust") == {
        "count": 1,
        "primary_ids": [],
    }
    qgis["errors"][0]["id"] = 2
    assert not compare("line/must-not-have-dangles", rust, qgis)


def test_statistics() -> None:
    """Use linearly interpolated IQRs and exclude failed samples."""
    assert quantile([1, 2, 3, 4], 0.25) == 1.75
    assert quantile([1, 2, 3, 4], 0.75) == 3.25
    assert summary([{"status": "timeout"}]) is None
    stats = summary([{"status": "ok", "seconds": 2}, {"status": "timeout"}])
    assert stats is not None and stats["median_s"] == 2
    with pytest.raises(ValueError):
        quantile([], 0.5)


def test_failure(tmp_path: Path) -> None:
    """Timeouts and crashes remain explicit instead of synthetic timing values."""

    class Failed:
        def sample(self, rule: str, timeout: float) -> dict:
            raise EngineFailure("timeout", "validation exceeded deadline")

    assert capture(Failed(), "x", 1)["status"] == "timeout"

    class Alive:
        def poll(self) -> None:
            return None

    with pytest.raises(EngineFailure, match="Deadline exceeded"):
        wait_file(tmp_path / "missing", cast(subprocess.Popen, Alive()), 0.001)
    process = subprocess.Popen([sys.executable, "-c", "raise SystemExit(3)"])
    process.wait()
    with pytest.raises(EngineFailure, match="Engine exited"):
        wait_file(tmp_path / "missing", process, 1)


def test_alternation_and_stop(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Failed engines stop; the healthy engine completes alternating samples."""
    calls = []

    class Rust:
        def __init__(self, *args: object) -> None:
            self.metadata = {"threads": "1"}

        def sample(self, rule: str, timeout: float) -> dict:
            calls.append("rust")
            return {"status": "ok", "seconds": 0.1, "errors": [], "raw_count": 0}

        def close(self) -> None:
            pass

    class Qgis(Rust):
        def load(self, *args: object) -> None:
            pass

        def sample(self, rule: str, timeout: float) -> dict:
            calls.append("qgis")
            raise EngineFailure("timeout", "deadline")

    monkeypatch.setattr("run.RustSession", Rust)
    args = Namespace(rust=Path("x"), startup_timeout=1, timeout=1)
    from run import QgisSession

    result = measure(
        "point/must-not-overlap", {}, args, tmp_path, cast(QgisSession, Qgis()), 3, 0
    )
    assert calls == ["rust", "qgis", "rust", "rust"]
    assert [s["status"] for s in result["qgis"]] == ["timeout", "not_run", "not_run"]
    assert [s["order"] for s in result["rust"]] == [0, 1, 0]


def test_rust_preloaded_protocol(tmp_path: Path) -> None:
    """The actual importer preserves IDs and fresh samples report stable results."""
    from run import ROOT, RustSession

    binary = ROOT / "target/release/examples/qgis_benchmark"
    if not binary.is_file():
        pytest.skip("Build the release benchmark example before integration checks")
    fixture = FIXTURES["point/must-not-overlap"]["violation"]
    files = write_case(tmp_path, fixture["input"], None)
    session = RustSession(binary, "point/must-not-overlap", files, tmp_path, 10)
    try:
        assert session.metadata["threads"] == "1"
        for _ in range(2):
            result = session.sample("point/must-not-overlap", 10)
            assert result["status"] == "ok"
            assert result["raw_count"] == 1
            assert result["errors"][0]["id"] == 1
            assert result["errors"][0]["destination"] == 2
    finally:
        session.close()


def test_audit_fixture_guard(tmp_path: Path) -> None:
    """Added probes can preserve timings only when timed fixture hashes reproduce."""
    from audit import verify_workloads

    rule = "point/must-not-overlap"
    primary, secondary, bad_blocks = workload(rule, 1000)
    files = write_case(tmp_path, primary, secondary)
    data = {
        "rules": [
            {
                "rule": rule,
                "workloads": [
                    {"size": 1000, "invalid_blocks": bad_blocks, "fixture": files}
                ],
            }
        ]
    }
    assert verify_workloads(data) == 1
    files["input"]["sha256"] = "tampered"
    with pytest.raises(ValueError, match="Timed fixture changed"):
        verify_workloads(data)


def test_publish(tmp_path: Path) -> None:
    """Regenerate SVG and Markdown only from recorded, eligible measurements."""
    samples = [
        {"status": "ok", "seconds": x, "raw_count": 1} for x in (0.01, 0.02, 0.03)
    ]
    record = {
        "rule": "point/must-not-overlap",
        "mapping": MAPPINGS["point/must-not-overlap"],
        "probes": [],
        "workloads": [
            {
                "size": 10000,
                "fixture": {},
                "paired_eligible": True,
                "rust": samples,
                "qgis": samples,
                "failures": {},
                "workload_agrees": True,
            }
        ],
    }
    path = tmp_path / "results.json"
    path.write_text(json.dumps({"metadata": {"started_at": "test"}, "rules": [record]}))
    publish(path, tmp_path)
    svg = (tmp_path / "comparison.svg").read_text()
    assert "Duplicate points" in svg
    assert "Validation runtime (ms, log scale)" in svg
    assert "Topology rule" in svg
    assert "lower is better" not in svg
    assert "correctness-gated" not in svg
    assert "QGIS Topology Checker" not in svg
    assert "Rust (1 thread)" in svg
    assert "QGIS 4.2.2" in svg
    assert "10.000" not in (tmp_path / "README.md").read_text()
    record["workloads"][0]["paired_eligible"] = False
    path.write_text(json.dumps({"metadata": {"started_at": "test"}, "rules": [record]}))
    publish(path, tmp_path)
    assert "Duplicate points" not in (tmp_path / "comparison.svg").read_text()
