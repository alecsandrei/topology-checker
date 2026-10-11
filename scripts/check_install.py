"""Smoke-test a regular wheel installation, optionally without GeoPandas."""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from importlib.metadata import distribution
from pathlib import Path

import topology_checker
from shapely.geometry import LineString, Point
from topology_checker import Rule, Topology, _native, explode_lines, merge_lines


def check_install(without_geopandas: bool = False) -> None:
    """Reject editable/source imports and exercise the base Shapely API."""
    installed = distribution("topology-checker")
    package = Path(topology_checker.__file__).resolve()
    prefix = Path(sys.prefix).resolve()
    if not package.is_relative_to(prefix):
        raise SystemExit(f"Import did not come from the test environment: {package}")
    if (
        package
        != Path(str(installed.locate_file("topology_checker/__init__.py"))).resolve()
    ):
        raise SystemExit("Imported package does not belong to the installed wheel")
    direct_url = json.loads(installed.read_text("direct_url.json") or "{}")
    if direct_url.get("dir_info", {}).get("editable", False):
        raise SystemExit("Tests require a regular wheel installation")
    native_path = Path(_native.__file__).resolve()  # pyright: ignore[reportAttributeAccessIssue]
    if not native_path.is_relative_to(package.parent):
        raise SystemExit(
            f"Native extension is outside the installed package: {native_path}"
        )
    if without_geopandas and importlib.util.find_spec("geopandas") is not None:
        raise SystemExit("Base installation unexpectedly includes GeoPandas")
    topology = Topology()
    topology.add_rule(Rule.MUST_NOT_OVERLAP, [Point(0, 0), Point(0, 0)])
    report = topology.validate()
    assert report.error_count == 1
    assert report.to_dict()["rules"]
    lines = [LineString([(0, 0), (1, 0)]), LineString([(1, 0), (2, 0)])]
    assert len(explode_lines(merge_lines(lines))) == 2
    print(f"Tested installed topology-checker {installed.version}: {package}")


def main() -> None:
    """Check the installed wheel and optional base dependency constraint."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--without-geopandas", action="store_true")
    args = parser.parse_args()
    check_install(args.without_geopandas)


if __name__ == "__main__":
    main()
