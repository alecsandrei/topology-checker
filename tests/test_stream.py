"""Bounded stream behavior, snapshot isolation and eager equivalence."""

from __future__ import annotations

import time
from typing import cast

import pytest
from shapely.geometry import MultiPoint, Point
from topology_checker import ReportSummary, Rule, Topology

from .test_api import INVALID_CASES


@pytest.mark.parametrize("batch_size", [1, 3, 1024])
@pytest.mark.parametrize("canonical,rule,data,other", INVALID_CASES)
def test_stream_matches_eager(
    canonical: str, rule: Rule, data: object, other: object, batch_size: int
) -> None:
    topology = Topology()
    topology.add_rule(rule, data, other=other)
    eager = topology.validate()
    with topology.iter_validate(batch_size) as stream:
        assert stream.summary is None
        violations = tuple(stream)
    assert [v.to_dict() for v in violations] == [v.to_dict() for v in eager.violations]
    summary = cast(ReportSummary | None, stream.summary)
    assert summary is not None
    assert summary.error_count == eager.error_count
    assert summary.warnings == eager.warnings
    assert [r.counts for r in summary.rules] == [r.counts for r in eager.rules]
    assert summary.is_valid == eager.is_valid


def test_stream_duplicate_pairs_multipart_and_snapshot() -> None:
    topology = Topology()
    layer = topology.add_layer(
        [MultiPoint([(0, 0), (1, 1)]), MultiPoint([(0, 0), (1, 1)]), Point(0, 0)],
        fids=[9, -4, 77],
    )
    topology.add_rule(Rule.MUST_NOT_OVERLAP, layer)
    with topology.iter_validate(2) as stream:
        topology.add_rule(Rule.MUST_NOT_OVERLAP, layer)
        result = tuple(stream)
    assert len(result) == 3
    assert result[0].origin is not None and result[0].origin.fid == -4
    assert result[1].origin is not None and result[1].origin.part is None
    assert stream.summary is not None and len(stream.summary.rules) == 1
    assert topology.validate().error_count == 6


def test_stream_early_close_and_consumer_failure() -> None:
    topology = Topology()
    topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(0, 0)] * 10000)
    started = time.monotonic()
    stream = topology.iter_validate(7)
    next(stream)
    stream.close()
    stream.close()
    assert stream.summary is None and list(stream) == []
    assert time.monotonic() - started < 3
    with pytest.raises(LookupError, match="consumer"):
        with topology.iter_validate(2) as stream:
            next(stream)
            raise LookupError("consumer")
    assert stream.summary is None


@pytest.mark.parametrize("size", [0, -1])
def test_stream_rejects_invalid_batch_size(size: int) -> None:
    with pytest.raises(ValueError, match="positive"):
        Topology().iter_validate(size)
