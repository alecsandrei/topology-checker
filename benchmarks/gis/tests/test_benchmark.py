"""Check comparison integrity without requiring external GIS installations."""

from pathlib import Path

import pytest

from benchmarks.gis.catalog import ENGINES, SHARED_RULES, SUPPORT
from benchmarks.gis.common import eligibility, normalize, sample_rounds
from benchmarks.gis.plot import publish, summary


def result(seconds: float = 0.01, ids: tuple[int, ...] = ()) -> dict:
    """Build one deterministic fake engine response."""
    return {
        "status": "ok",
        "seconds": seconds,
        "raw_count": len(ids),
        "errors": [{"kind": "outside", "id": fid, "layer": 0} for fid in ids],
    }


def test_catalog_coverage() -> None:
    """Every OSS engine exposes the same headline checks."""
    for engine in ENGINES:
        assert set(SHARED_RULES) <= set(SUPPORT[engine])
    assert set(SUPPORT["postgis"]) == set(SUPPORT["qgis"]) | set(SUPPORT["gvsig"])


def test_normalization_retains_identity() -> None:
    """Equal counts with different features must never pass the gate."""
    rule = SHARED_RULES[0]
    assert normalize(rule, result(ids=(1,))) != normalize(rule, result(ids=(2,)))
    errors = result(ids=(1, 1))
    assert normalize("line/must-not-have-dangles", errors)["count"] == 1
    errors["errors"].append({"kind": "outer-boundary", "id": None})
    assert normalize("polygon/must-not-have-gaps", errors)["count"] == 2


def test_independent_eligibility() -> None:
    """One competitor failing does not invalidate another competitor."""
    assert eligibility(True, True, [result(), result()], 2) == (True, "paired")
    assert not eligibility(False, True, [result(), result()], 2)[0]
    assert not eligibility(True, False, [result(), result()], 2)[0]
    assert not eligibility(True, True, [result()], 2)[0]


def test_rotating_order_and_stop() -> None:
    """Healthy engines continue after one engine fails, without hidden retries."""
    calls: list[str] = []

    class Session:
        def __init__(self, name: str) -> None:
            self.name = name
            self.metadata = {}

        def sample(self, rule: str, timeout: float) -> dict:
            calls.append(self.name)
            if self.name == "grass":
                return {"status": "timeout", "error": "deadline"}
            return result()

        def close(self) -> None:
            pass

    sessions = {name: Session(name) for name in ("rust", "qgis", "grass")}
    measured = sample_rounds(SHARED_RULES[0], sessions, 2, 1, 1)
    assert calls == ["rust", "qgis", "grass", "qgis", "rust", "rust", "qgis"]
    assert [s["status"] for s in measured["samples"]["grass"]] == ["not_run", "not_run"]
    assert len(measured["samples"]["rust"]) == 2


def test_report_keeps_missing_tools_visible(tmp_path: Path) -> None:
    """A failed or unsupported engine cannot silently disappear from the figure."""
    data = {
        "schema_version": 2,
        "metadata": {"platform": "test", "started_at": "test"},
        "engines": {name: {"version": "test"} for name in ("rust", *ENGINES)},
        "rules": [],
    }
    for rule in SHARED_RULES:
        data["rules"].append(
            {
                "rule": rule,
                "probes": [],
                "workloads": [
                    {
                        "size": 10000,
                        "fixture": {},
                        "samples": {"rust": [result()], "qgis": [result()]},
                        "comparisons": {
                            name: {"eligible": name == "qgis", "reason": "unavailable"}
                            for name in ENGINES
                        },
                    }
                ],
            }
        )
    publish(data, tmp_path)
    svg = (tmp_path / "comparison.svg").read_text()
    for label in ("topology-checker", "QGIS", "GRASS", "PostGIS", "gvSIG"):
        assert label in svg
    assert "unavailable" in svg
    assert not (tmp_path / "publishable.json").exists()
    assert (tmp_path / "samples.csv").is_file()
    assert summary([{"status": "timeout"}]) is None
    with pytest.raises(ValueError):
        summary([result(seconds=-1)])
