"""Deterministic synthetic workloads and actual C++ Topology Checker mappings."""

import hashlib
import json
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
CATALOG = json.loads((HERE / "fixtures.json").read_text())
FIXTURES = {item["rule"]: item for item in CATALOG["fixtures"]}
MAPPINGS = {
    "point/must-not-overlap": (
        "must not have duplicates",
        "Pairs agree; triple duplicates produce n-1 QGIS rows versus all Rust pairs.",
    ),
    "point/must-be-inside": (
        "must be inside",
        "Strict containment; zero tolerance. Boundary points fail.",
    ),
    "line/must-not-have-dangles": (
        "must not have dangles",
        "QGIS checks original endpoints, Rust nodes interiors. Compare affected feature IDs only.",
    ),
    "line/must-not-have-pseudo-nodes": (
        "must not have pseudos",
        "QGIS checks original endpoints; closed lines and interior noding require probes.",
    ),
    "polygon/must-not-overlap": (
        "must not overlap",
        "QGIS strict overlaps excludes equal and contained polygons; Rust reports any positive-area intersection. Only first QGIS IDs are exposed.",
    ),
    "polygon/must-not-overlap-with": (
        "must not overlap with",
        "QGIS strict overlaps excludes equal and contained polygons; Rust reports any positive-area intersection. Compare primary IDs and counts.",
    ),
    "polygon/must-not-have-gaps": (
        "must not have gaps",
        "Exclude Rust outer-boundary records; compare gap counts, not unavailable QGIS provenance.",
    ),
    "geometry/must-not-be-multipart": (
        "must not have multi-part geometries",
        "Both test more than one part, including single-part Multi probes.",
    ),
    "point/must-be-covered-by-lines": (
        "must be covered by",
        "Related: QGIS uses touches, so line interiors fail whereas endpoints pass.",
    ),
    "point/must-be-covered-by-line-endpoints": (
        "must be covered by endpoints of",
        "Open endpoints; closed-line semantics require probes.",
    ),
    "point/must-be-covered-by-polygon-boundaries": (
        "must be covered by",
        "QGIS touches on polygons checks boundaries; test exterior and shared edges.",
    ),
    "line/endpoints-must-be-covered-by-points": (
        "end points must be covered by",
        "Normalize endpoint rows to affected primary feature IDs; closed lines require probes.",
    ),
    "polygon/must-contain-point": (
        "must contain",
        "Strict polygon contains point; boundary points fail.",
    ),
}
# Additional cases exercise implementation differences beyond the original draft.
CATALOG["edge_cases"] += [
    {
        "rule": "line/endpoints-must-be-covered-by-points",
        "name": "closed line without point at ring start",
        "input": FIXTURES["line/must-not-have-dangles"]["clean"]["input"],
        "other": [{"type": "Point", "coordinates": [30, 30]}],
        "expected": 0,
    },
    {
        "rule": "polygon/must-not-overlap",
        "name": "equal polygons",
        "input": FIXTURES["polygon/must-contain-point"]["clean"]["input"] * 2,
        "expected": 1,
    },
    {
        "rule": "polygon/must-not-overlap",
        "name": "contained polygon",
        "input": [
            {
                "type": "Polygon",
                "coordinates": [[[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]]],
            },
            {
                "type": "Polygon",
                "coordinates": [[[2, 2], [4, 2], [4, 4], [2, 4], [2, 2]]],
            },
        ],
        "expected": 1,
    },
    {
        "rule": "polygon/must-not-overlap-with",
        "name": "equal polygons",
        "input": FIXTURES["polygon/must-contain-point"]["clean"]["input"],
        "other": FIXTURES["polygon/must-contain-point"]["clean"]["input"],
        "expected": 1,
    },
    {
        "rule": "polygon/must-not-overlap-with",
        "name": "contained polygon",
        "input": [
            {
                "type": "Polygon",
                "coordinates": [[[2, 2], [4, 2], [4, 4], [2, 4], [2, 2]]],
            }
        ],
        "other": FIXTURES["polygon/must-contain-point"]["clean"]["input"],
        "expected": 1,
    },
    {
        "rule": "line/must-not-have-dangles",
        "name": "interior-noded connection across closed ring",
        "input": [
            {
                "type": "LineString",
                "coordinates": [[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]],
            },
            {"type": "LineString", "coordinates": [[0, 5], [10, 5]]},
        ],
        "expected": 0,
    },
    {
        "rule": "point/must-be-covered-by-polygon-boundaries",
        "name": "point on shared polygon edge",
        "input": [{"type": "Point", "coordinates": [10, 5]}],
        "other": [
            {
                "type": "Polygon",
                "coordinates": [[[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]]],
            },
            {
                "type": "Polygon",
                "coordinates": [[[10, 0], [20, 0], [20, 10], [10, 10], [10, 0]]],
            },
        ],
        "expected": 0,
    },
    {
        "rule": "line/must-not-have-dangles",
        "name": "closed line",
        "input": FIXTURES["line/must-not-have-dangles"]["clean"]["input"],
        "expected": 0,
    },
    {
        "rule": "line/must-not-have-pseudo-nodes",
        "name": "closed line",
        "input": FIXTURES["line/must-not-have-dangles"]["clean"]["input"],
        "expected": 0,
    },
    {
        "rule": "line/endpoints-must-be-covered-by-points",
        "name": "both endpoints uncovered",
        "input": FIXTURES["line/endpoints-must-be-covered-by-points"]["clean"]["input"],
        "other": [{"type": "Point", "coordinates": [30, 30]}],
        "expected": 1,
    },
    {
        "rule": "point/must-be-covered-by-line-endpoints",
        "name": "closed line start",
        "input": [{"type": "Point", "coordinates": [0, 0]}],
        "other": FIXTURES["line/must-not-have-dangles"]["clean"]["input"],
        "expected": 1,
    },
    {
        "rule": "polygon/must-contain-point",
        "name": "boundary point",
        "input": FIXTURES["polygon/must-contain-point"]["clean"]["input"],
        "other": [{"type": "Point", "coordinates": [0, 2]}],
        "expected": 1,
    },
    {
        "rule": "polygon/must-not-overlap",
        "name": "edge contact",
        "input": [
            {
                "type": "Polygon",
                "coordinates": [[[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]]],
            },
            {
                "type": "Polygon",
                "coordinates": [[[10, 0], [20, 0], [20, 10], [10, 10], [10, 0]]],
            },
        ],
        "expected": 0,
    },
]


def translate(value: list, x: float, y: float) -> list:
    """Translate nested GeoJSON coordinates without mutating the source."""
    if isinstance(value[0], (float, int)):
        return [value[0] + x, value[1] + y]
    return [translate(child, x, y) for child in value]


def coordinate_count(value: list) -> int:
    """Count XY coordinate pairs, including closing ring coordinates."""
    return (
        1
        if isinstance(value[0], (float, int))
        else sum(coordinate_count(v) for v in value)
    )


def collection(geometries: list[dict], name: str) -> dict:
    """Assign stable positive IDs shared by OGR and the memory provider."""
    return {
        "type": "FeatureCollection",
        "name": name,
        "crs": {"type": "name", "properties": {"name": "EPSG:3857"}},
        "features": [
            {"type": "Feature", "id": i, "properties": {"bench_id": i}, "geometry": g}
            for i, g in enumerate(geometries, 1)
        ],
    }


def workload(rule: str, size: int) -> tuple[list[dict], list[dict] | None, int]:
    """Build exactly size primary features with every hundredth block invalid."""
    if size < 1:
        raise ValueError("size must be positive")
    fixture = FIXTURES[rule]
    width = max(len(fixture["clean"]["input"]), len(fixture["violation"]["input"]))
    primary: list[dict] = []
    secondary: list[dict] | None = [] if fixture["clean"]["other"] is not None else None
    bad_blocks = 0
    block = 0
    while len(primary) < size:
        invalid = block % 100 == 99 and size - len(primary) >= width
        case = fixture["violation" if invalid else "clean"]
        bad_blocks += int(invalid)
        x, y = (block % 1000) * 100, (block // 1000) * 100
        # Pad a smaller clean pattern by moving additional independent copies.
        geometries = case["input"]
        for i in range(min(width, size - len(primary))):
            g = geometries[i % len(geometries)]
            primary.append(
                {
                    "type": g["type"],
                    "coordinates": translate(
                        g["coordinates"], x + (i // len(geometries)) * 30, y
                    ),
                }
            )
        if secondary is not None:
            for g in case["other"]:
                secondary.append(
                    {
                        "type": g["type"],
                        "coordinates": translate(g["coordinates"], x, y),
                    }
                )
        block += 1
    return primary, secondary, bad_blocks


def write_case(
    directory: Path, primary: list[dict], secondary: list[dict] | None
) -> dict[str, Any]:
    """Save fixtures, hashes, feature counts and coordinate counts."""
    directory.mkdir(parents=True, exist_ok=True)
    result: dict[str, Any] = {}
    for name, geometries in (("input", primary), ("other", secondary)):
        if geometries is None:
            continue
        data = (
            json.dumps(collection(geometries, name), separators=(",", ":")) + "\n"
        ).encode()
        path = directory / f"{name}.geojson"
        path.write_bytes(data)
        result[name] = {
            # Repo-relative, so results do not record the local checkout path.
            "path": str(path.resolve().relative_to(ROOT, walk_up=True)),
            "sha256": hashlib.sha256(data).hexdigest(),
            "features": len(geometries),
            "coordinates": sum(coordinate_count(g["coordinates"]) for g in geometries),
        }
    return result
