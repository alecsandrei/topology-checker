"""Check result identity, complete publication gates and minimal graph structure."""

from copy import deepcopy
from pathlib import Path
from typing import Any
from unittest.mock import patch

import pytest
from matplotlib.axes import Axes

from benchmarks.gis.catalog import ENGINES, HEADLINE_RULES, LABELS, SUPPORT
from benchmarks.gis.common import (
    capture,
    eligibility,
    fingerprint,
    normalize,
    sample_rounds,
)
from benchmarks.gis.plot import headline_size, publish, publishable, summary
from benchmarks.gis.probes import additional_cases, polygon
from benchmarks.gis.run import probe_cases


def result(seconds: float = 0.01, ids: tuple[int, ...] = ()) -> dict[str, Any]:
    """Build a deterministic fake engine response."""
    return {
        "status": "ok",
        "seconds": seconds,
        "raw_count": len(ids),
        "errors": [{"kind": "outside", "id": fid, "layer": 0} for fid in ids],
    }


def complete_data() -> dict[str, Any]:
    """Make a complete native CI report with all provenance and combinations."""
    return {
        "metadata": {
            "source_revision": "test",
            "source_tree_sha256": "test",
            "tooling_sha256": {"test": "test"},
            "container_image": "test",
            "rust_binary_sha256": "test",
            "packages": "test",
            "repetitions": 10,
            "warmups": 2,
            "execution": {"native_linux_ci": True, "cpu_affinity": [0]},
        },
        "engines": {name: {"version": "test"} for name in ("rust", *ENGINES)},
        "rules": [
            {
                "rule": rule,
                "probes": [],
                "workloads": [
                    {
                        "size": size,
                        "samples": {
                            name: [fingerprint(rule, result()) for _ in range(10)]
                            for name in ("rust", *ENGINES)
                        },
                        "warmups": [
                            {"engine": name, **fingerprint(rule, result())}
                            for name in ("rust", *ENGINES)
                            for _ in range(2)
                        ],
                        "comparisons": {
                            name: {"eligible": size == 1000, "reason": "paired"}
                            for name in ENGINES
                        },
                    }
                    for size in (1000, 10000)
                ],
            }
            for rule in HEADLINE_RULES
        ],
    }


def test_catalog_coverage() -> None:
    """All engines expose all six checks using validation workflows."""
    assert len(HEADLINE_RULES) == 6
    assert set(LABELS) == {"rust", *ENGINES} and len(LABELS) == 4
    for engine in ENGINES:
        assert set(HEADLINE_RULES) <= set(SUPPORT[engine])
        assert all("repair" not in SUPPORT[engine][rule] for rule in HEADLINE_RULES)
    assert set(SUPPORT["postgis"]) == set(SUPPORT["qgis"]) | {
        "line/must-be-inside",
        "line/must-not-intersect",
        "line/must-not-intersect-with",
        "line/must-not-overlap-with",
    }
    assert "fme" not in SUPPORT


def test_pairs_preserve_both_ids() -> None:
    """Equal counts and primary IDs cannot disguise different pairs."""
    first, second = result(ids=(1,)), result(ids=(1,))
    first["errors"][0]["destination"] = 2
    second["errors"][0]["destination"] = 3
    for rule in ("point/must-not-overlap", "polygon/must-not-overlap"):
        assert normalize(rule, first) != normalize(rule, second)
        first["errors"].append({"kind": "overlap", "id": 2, "destination": 1})
        assert normalize(rule, first)["pairs"] == [[1, 2]]
        first["errors"].pop()
    assert normalize(HEADLINE_RULES[0], result(ids=(1,))) != normalize(
        HEADLINE_RULES[0], result(ids=(2,))
    )
    assert normalize("line/must-not-have-dangles", result(ids=(1, 1)))["count"] == 1


def test_gaps_compare_geometry_and_islands() -> None:
    """Equivalent rings normalize together; moved gaps and filled islands differ."""
    outer = polygon(2, 2, 6)
    island = polygon(4, 4, 2)
    full = deepcopy(outer)
    full["coordinates"].append(island["coordinates"][0][::-1])
    native = {"errors": [{"kind": "gap", "geometry": full}]}
    rust = {
        "errors": [
            {
                "kind": "gap",
                "geometry": {
                    "type": "LineString",
                    "coordinates": outer["coordinates"][0],
                },
            },
            {
                "kind": "outer-boundary",
                "geometry": {
                    "type": "LineString",
                    "coordinates": island["coordinates"][0],
                },
            },
        ]
    }
    rule = "polygon/must-not-have-gaps"
    assert normalize(rule, native) == normalize(rule, rust)
    rust["errors"].pop()
    assert normalize(rule, native) != normalize(rule, rust)
    assert normalize(rule, {"errors": []})["count"] == 0
    assert normalize(
        rule, {"errors": [{"kind": "gap", "geometry": polygon(20, 20, 6)}]}
    ) != normalize(rule, rust)


def test_edge_case_fixtures() -> None:
    """Keep connectivity, pair, boundary, bounded-hole and containment probes."""
    assert additional_cases("point/must-not-overlap")[0]["identity"]["pairs"] == [
        [1, 2],
        [1, 3],
        [2, 3],
    ]
    names = {case["name"] for rule in HEADLINE_RULES for case in probe_cases(rule)}
    assert {
        "equal polygons",
        "contained polygon",
        "boundary touch",
        "bounded hole with island",
        "closed line",
        "T junction with interior connection",
        "point in hole",
        "point on hole boundary",
    } <= names


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
            self.name, self.metadata = name, {}

        def sample(self, rule: str, timeout: float) -> dict:
            calls.append(self.name)
            return (
                {"status": "timeout", "error": "deadline"}
                if self.name == "grass"
                else result()
            )

        def close(self) -> None:
            pass

    measured = sample_rounds(
        HEADLINE_RULES[0],
        {name: Session(name) for name in ("rust", "qgis", "grass")},
        2,
        1,
        1,
    )
    assert calls == ["rust", "qgis", "grass", "qgis", "rust", "rust", "qgis"]
    assert [s["status"] for s in measured["samples"]["grass"]] == ["not_run", "not_run"]
    assert len(measured["samples"]["rust"]) == 2
    assert summary([{"status": "timeout"}]) is None
    with pytest.raises(ValueError):
        summary([result(seconds=-1)])
    with patch.object(Session, "sample", return_value=result(seconds=0)):
        assert capture(Session("rust"), HEADLINE_RULES[0], 1)["status"] == "failed"


@pytest.mark.parametrize("rule", HEADLINE_RULES)
@pytest.mark.parametrize("engine", ("rust", *ENGINES))
def test_every_combination_required(rule: str, engine: str) -> None:
    """Removing any of the 24 combinations blocks the graph and publication."""
    data = complete_data()
    work = next(record for record in data["rules"] if record["rule"] == rule)[
        "workloads"
    ][0]
    work["samples"][engine].pop()
    assert headline_size(data) == 0
    assert not publishable(data)


def test_common_size_and_provenance() -> None:
    """Checks at different sizes or with missing provenance cannot be published."""
    data = complete_data()
    assert headline_size(data) == 1000 and publishable(data)
    data["rules"][0]["workloads"][0]["comparisons"]["qgis"]["eligible"] = False
    data["rules"][0]["workloads"][1]["comparisons"] = {
        name: {"eligible": True} for name in ENGINES
    }
    assert headline_size(data) == 0
    for field in (
        "source_revision",
        "source_tree_sha256",
        "container_image",
        "tooling_sha256",
        "rust_binary_sha256",
        "packages",
    ):
        data = complete_data()
        del data["metadata"][field]
        assert not publishable(data)
    data = complete_data()
    data["metadata"]["execution"]["cpu_affinity"] = [0, 1]
    assert not publishable(data)


def test_minimal_graph_and_failure_report(tmp_path: Path) -> None:
    """Four bar series, six groups and a four-tool legend; failures stay in reports."""
    data = complete_data()
    with patch.object(Axes, "bar", autospec=True, wraps=Axes.bar) as bar:
        publish(data, tmp_path)
        assert bar.call_count == 4
        assert all(len(call.args[1]) == 6 for call in bar.call_args_list)
    # Render with the real artist as well to verify SVG text and plot count.
    publish(data, tmp_path)
    svg = (tmp_path / "comparison.svg").read_text()
    assert svg.count('id="axes_') == 1
    for label in LABELS.values():
        assert label in svg
    assert "Median validation time (ms)" in svg
    assert "Same data" not in svg and "primary and" not in svg
    assert "LineCollection" not in svg
    assert (tmp_path / "publishable.json").is_file()
    data["rules"][0]["workloads"][0]["comparisons"]["grass"] = {
        "eligible": False,
        "reason": "timeout",
    }
    publish(data, tmp_path)
    assert not (tmp_path / "comparison.svg").exists()
    assert not (tmp_path / "publishable.json").exists()
    report = (tmp_path / "README.md").read_text()
    assert "timeout" in report and "all 24 comparisons" in report
    assert (tmp_path / "samples.csv").is_file()


def test_probes_start_fresh_sessions(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """Every probe starts and closes its own session for each engine."""
    from argparse import Namespace

    from benchmarks.gis.run import run_probes

    calls: list[str] = []

    class FakeSession:
        metadata: dict[str, Any] = {"version": "test"}

        def sample(self, rule: str, timeout: float) -> dict[str, Any]:
            calls.append("sample")
            return result()

        def close(self) -> None:
            calls.append("close")

    def start(engine: str, rule: str, fixture: dict[str, Any], *args: Any) -> Any:
        assert Path(fixture["input"]["path"]).name == "input.geojson"
        calls.append("start:" + engine)
        return FakeSession()

    cases = [{"name": "clean", "input": [], "other": None, "expected": 0}] * 2
    monkeypatch.setattr("benchmarks.gis.run.start", start)
    monkeypatch.setattr("benchmarks.gis.run.probe_cases", lambda rule: cases)
    records, gates = run_probes(
        HEADLINE_RULES[0],
        tmp_path,
        Namespace(engines=["grass"], timeout=1, startup_timeout=1),
        {"engines": {"rust": {"status": "ready"}, "grass": {"status": "ready"}}},
    )
    assert len(records) == 2 and gates == {"grass": True}
    assert calls.count("start:rust") == calls.count("start:grass") == 2
    assert calls.count("sample") == calls.count("close") == 4


def test_malformed_results_fail_explicitly() -> None:
    """Missing pair provenance and missing gap geometry are failures, not crashes."""

    class Malformed:
        metadata: dict[str, Any] = {}

        def sample(self, rule: str, timeout: float) -> dict[str, Any]:
            return result(ids=(1,))

        def close(self) -> None:
            pass

    for rule in ("point/must-not-overlap", "polygon/must-not-have-gaps"):
        assert capture(Malformed(), rule, 1)["status"] == "failed"


def test_warmups_and_recorded_results_required() -> None:
    """Incomplete warmups or differing result hashes withhold the graph."""
    data = complete_data()
    data["rules"][0]["workloads"][0]["warmups"].pop()
    assert headline_size(data) == 0
    data = complete_data()
    data["rules"][0]["workloads"][0]["samples"]["grass"][0]["result_sha256"] = (
        "different"
    )
    assert headline_size(data) == 0
    data = complete_data()
    data["rules"][0]["workloads"][0]["warmups"][0]["status"] = "timeout"
    assert headline_size(data) == 0


def test_gap_wkb_preserves_translated_coordinates() -> None:
    """Binary geometry extraction keeps translated integer coordinates unchanged."""
    from shapely import to_wkb
    from shapely.geometry import shape

    geometry = polygon(29902, 2, 2)
    expected = {"errors": [{"kind": "gap", "geometry": geometry}]}
    binary = {
        "errors": [{"kind": "gap", "geometry_wkb": to_wkb(shape(geometry)).hex()}]
    }
    assert normalize("polygon/must-not-have-gaps", binary) == normalize(
        "polygon/must-not-have-gaps", expected
    )
