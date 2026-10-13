"""Opt-in fixture comparisons using actual native GIS runtimes."""

import os
from pathlib import Path

import pytest

from benchmarks.gis.adapters import create_session
from benchmarks.gis.catalog import ENGINES, HEADLINE_RULES
from benchmarks.gis.common import normalize
from benchmarks.gis.fixtures import write_case
from benchmarks.gis.run import probe_cases
from benchmarks.gis.sessions import RustSession

pytestmark = pytest.mark.skipif(
    os.environ.get("GIS_INTEGRATION") != "1",
    reason="Run in the benchmark container with GIS_INTEGRATION=1",
)


def test_rust_preloaded_protocol(tmp_path: Path) -> None:
    """The production importer preserves IDs across fresh timed samples."""
    from benchmarks.gis.fixtures import FIXTURES, ROOT

    binary = Path(
        os.environ.get(
            "GIS_RUST_BINARY", str(ROOT / "target/release/examples/qgis_benchmark")
        )
    )
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


@pytest.mark.parametrize("engine", ENGINES)
@pytest.mark.parametrize("rule", HEADLINE_RULES)
def test_native_engine_probes(engine: str, rule: str, tmp_path: Path) -> None:
    """Compare every semantic fixture with Rust, including complete result identity."""
    if engine not in os.environ.get("GIS_TEST_ENGINES", ",".join(ENGINES)).split(","):
        pytest.skip("Engine not selected")
    for index, case in enumerate(probe_cases(rule)):
        directory = tmp_path / str(index)
        files = write_case(directory, case["input"], case.get("other"))
        actual = {}
        for tool in ("rust", engine):
            session = create_session(tool, rule, files, directory / tool, 180)
            try:
                response = session.sample(rule, 60)
                assert response["status"] == "ok", (case["name"], response)
                assert response["seconds"] > 0
                actual[tool] = normalize(rule, response)
            finally:
                session.close()
        assert actual["rust"] == actual[engine], (case["name"], actual)
        if "expected" in case:
            assert actual["rust"]["count"] == case["expected"], case["name"]
        if "expected_gaps" in case:
            expected = normalize(
                rule,
                {
                    "errors": [
                        {"kind": "gap", "geometry": geometry}
                        for geometry in case["expected_gaps"]
                    ]
                },
            )
            assert actual["rust"] == expected, case["name"]
        for key, value in case.get("identity", {}).items():
            assert actual["rust"][key] == value


def test_qgis_gap_wkb_at_headline_size(tmp_path: Path) -> None:
    """Translated gaps must retain exact geometry at the published feature count."""
    from benchmarks.gis.fixtures import workload

    rule = "polygon/must-not-have-gaps"
    primary, secondary, _ = workload(rule, 1000)
    files = write_case(tmp_path, primary, secondary)
    results = {}
    for engine in ("rust", "qgis"):
        session = create_session(engine, rule, files, tmp_path / engine, 180)
        try:
            for _ in range(2):
                response = session.sample(rule, 60)
                results[engine] = normalize(rule, response)
                assert results[engine]["count"] == 10
                if engine == "qgis":
                    assert all("geometry_wkb" in error for error in response["errors"])
                    assert results[engine] == results["rust"]
        finally:
            session.close()
