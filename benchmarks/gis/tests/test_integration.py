"""Opt-in checks against actual engines in the isolated benchmark container."""

import json
import os
from pathlib import Path

import pytest

from benchmarks.gis.adapters import create_session
from benchmarks.gis.catalog import ENGINES, SHARED_RULES
from benchmarks.gis.common import normalize
from benchmarks.qgis.fixtures import FIXTURES, write_case

pytestmark = pytest.mark.skipif(
    os.environ.get("GIS_INTEGRATION") != "1",
    reason="Run inside the benchmark container with GIS_INTEGRATION=1",
)


@pytest.mark.parametrize("engine", ENGINES)
@pytest.mark.parametrize("rule", SHARED_RULES)
def test_stock_engine_containment(engine: str, rule: str, tmp_path: Path) -> None:
    """Require each native engine to distinguish clean and violating fixtures."""
    selected = os.environ.get("GIS_TEST_ENGINES", ",".join(ENGINES)).split(",")
    if engine not in selected:
        pytest.skip("Engine not selected for this local adapter check")
    for name in ("clean", "violation"):
        case = FIXTURES[rule][name]
        directory = tmp_path / name
        files = write_case(directory, case["input"], case["other"])
        session = create_session(engine, rule, files, directory / engine, 180)
        try:
            response = session.sample(rule, 60)
            assert response["status"] == "ok", response
            assert response["seconds"] > 0
            json.dumps({"metadata": session.metadata, "response": response})
            normalized = normalize(rule, response)
            if name == "clean":
                assert normalized["count"] == 0, response
            else:
                assert normalized["count"] > 0, response
                assert normalized["primary_ids"]
                assert all(
                    1 <= fid <= len(case["input"]) for fid in normalized["primary_ids"]
                )
        finally:
            session.close()
