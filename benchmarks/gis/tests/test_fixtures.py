"""Verify the shared rule catalog and deterministic benchmark workloads."""

import json
from pathlib import Path

import pytest

from benchmarks.gis.fixtures import (
    FIXTURES,
    MAPPINGS,
    collection,
    translate,
    workload,
    write_case,
)


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
