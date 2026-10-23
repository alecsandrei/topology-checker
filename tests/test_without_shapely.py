"""The WKB API works when shapely, pyproj and GeoPandas cannot be imported.

Each check runs in a fresh interpreter whose import system blocks the optional
packages, so it also proves that nothing imports them eagerly.
"""

from __future__ import annotations

import importlib.abc
import struct
import subprocess
import sys
from collections.abc import Callable, Sequence
from importlib.machinery import ModuleSpec
from pathlib import Path
from types import ModuleType

import pytest

ROOT = Path(__file__).resolve().parents[1]
OPTIONAL = ("shapely", "pyproj", "geopandas")


class Blocker(importlib.abc.MetaPathFinder):
    """Fail imports of the optional packages and record each attempt."""

    def __init__(self) -> None:
        self.attempts: list[str] = []

    def find_spec(
        self,
        fullname: str,
        path: Sequence[str] | None,
        target: ModuleType | None = None,
    ) -> ModuleSpec | None:
        if fullname.partition(".")[0] in OPTIONAL:
            self.attempts.append(fullname)
            raise ModuleNotFoundError(f"No module named {fullname!r}", name=fullname)
        return None


def point(x: float, y: float) -> bytes:
    return struct.pack("<BIdd", 1, 1, x, y)


def line(*coords: tuple[float, float]) -> bytes:
    data = struct.pack("<BII", 1, 2, len(coords))
    return data + b"".join(struct.pack("<dd", x, y) for x, y in coords)


def check_layers() -> None:
    from topology_checker import GeometryKind, Rule, Topology

    point_z = struct.pack(">BIddd", 0, 1001, 3, 4, 5)
    multi = struct.pack("<BII", 1, 4, 2) + struct.pack(
        "<BIdd", 1, 1, *[float("nan")] * 2
    )
    multi += point(3, 4)
    topology = Topology()
    id = topology.add_layer(
        [point(0, 0), bytearray(point_z), None, memoryview(multi)],
        fids=[10, 11, 12, 13],
        name="wells",
    )
    layer = topology.layer(id)
    assert layer.fids == (10, 11, 13)
    assert layer.kind is GeometryKind.POINT
    assert layer.wkb[1] == point(3, 4)
    assert layer.warnings == (
        "layer wells, FID 11: discarded extra dimensions",
        "layer wells, input 2, FID 12: skipped missing feature",
        "layer wells, FID 13: skipped 1 empty parts",
    )
    spec = topology.add_rule(Rule.MUST_NOT_OVERLAP, id)
    assert spec.resolved_rule.value == "point/must-not-overlap"
    try:
        topology.add_layer([b"\x01\x01"], fids=[4])
    except ValueError as error:
        assert str(error) == "layer layer_1, FID 4: invalid WKB: unexpected end of data"
    else:
        raise AssertionError("unreadable WKB must be rejected")
    assert len(topology.layers) == 1


def check_validate() -> None:
    from topology_checker import Rule, Topology, Violation

    assert isinstance(Violation.wkb, property)
    topology = Topology()
    spec = topology.add_rule(
        Rule.MUST_NOT_HAVE_DANGLES, [line((0, 0), (1, 0)), line((1, 0), (2, 0))]
    )
    report = topology.validate()
    assert report.error_count == 2
    assert [v.wkb for v in report.violations] == [point(0, 0), point(2, 0)]
    assert [(v.origin and v.origin.fid) for v in report.violations] == [0, 1]
    assert report.rules[0].spec is spec
    with topology.iter_validate(batch_size=1) as stream:
        streamed = [violation.wkb for violation in stream]
    assert streamed == [point(0, 0), point(2, 0)]
    assert stream.summary is not None and stream.summary.error_count == 2
    valid = Topology()
    valid.add_rule(Rule.MUST_NOT_OVERLAP, [point(0, 0), point(1, 1)])
    assert valid.validate().to_dict()["is_valid"]


def check_lines() -> None:
    from topology_checker import explode_lines, merge_lines

    data = [line((0, 0), (1, 0)), None, bytearray(line((1, 0), (2, 0), (2, 1)))]
    assert merge_lines(data) == (line((0, 0), (1, 0), (2, 0), (2, 1)),)
    assert explode_lines(data) == (
        line((0, 0), (1, 0)),
        line((1, 0), (2, 0)),
        line((2, 0), (2, 1)),
    )


def expect_import_error(action: Callable[[], object], extra: str) -> None:
    try:
        action()
    except ImportError as error:
        assert "topology-checker[io]" in str(error) and extra in str(error), error
    else:
        raise AssertionError("an ImportError naming the extra was expected")


def check_optional_features() -> None:
    from topology_checker import Rule, Topology

    topology = Topology()
    id = topology.add_layer([point(0, 0), point(0, 0)])
    topology.add_rule(Rule.MUST_NOT_OVERLAP, id)
    report = topology.validate()
    violation = report.violations[0]
    expect_import_error(lambda: violation.geometry, "shapely")
    expect_import_error(violation.to_dict, "shapely")
    expect_import_error(report.to_dict, "shapely")
    expect_import_error(lambda: topology.layer(id).geometries, "shapely")
    expect_import_error(report.to_geodataframe, "GeoDataFrame")
    expect_import_error(lambda: Topology().add_layer([], crs="EPSG:4326"), "CRS")
    expect_import_error(
        lambda: Topology().add_layer([{"type": "Point", "coordinates": [0, 0]}]),
        "shapely",
    )
    try:
        Topology().add_layer([object()])
    except TypeError:
        pass
    else:
        raise AssertionError("unsupported items stay a TypeError")
    assert len(topology.layers) == 1


CHECKS: dict[str, Callable[[], None]] = {
    "layers": check_layers,
    "validate": check_validate,
    "lines": check_lines,
    "optional-features": check_optional_features,
}


def run(name: str) -> None:
    """Run one check with the optional packages blocked."""
    assert not any(module.partition(".")[0] in OPTIONAL for module in sys.modules)
    blocker = Blocker()
    sys.meta_path.insert(0, blocker)
    import topology_checker

    assert topology_checker.Topology
    CHECKS[name]()
    loaded = [m for m in sys.modules if m.partition(".")[0] in OPTIONAL]
    assert not loaded, loaded
    if name != "optional-features":
        assert not blocker.attempts, blocker.attempts


@pytest.mark.parametrize("name", CHECKS)
def test_without_shapely(name: str) -> None:
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            f"from tests.test_without_shapely import run; run({name!r})",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
