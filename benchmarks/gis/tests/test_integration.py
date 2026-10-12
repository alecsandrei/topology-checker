"""Opt-in fixture comparisons using actual native GIS runtimes."""

import os
from pathlib import Path

import pytest

from benchmarks.gis.adapters import create_session
from benchmarks.gis.catalog import ENGINES, HEADLINE_RULES
from benchmarks.gis.common import normalize
from benchmarks.gis.run import probe_cases
from benchmarks.qgis.fixtures import write_case

pytestmark = pytest.mark.skipif(
    os.environ.get("GIS_INTEGRATION") != "1",
    reason="Run in the benchmark container with GIS_INTEGRATION=1",
)


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
