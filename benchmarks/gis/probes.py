"""Small correctness probes for pair identity, connectivity and bounded voids."""

from typing import Any


def point(x: float, y: float) -> dict[str, Any]:
    """Create one point geometry."""
    return {"type": "Point", "coordinates": [x, y]}


def line(*xy: tuple[float, float]) -> dict[str, Any]:
    """Create one line geometry."""
    return {"type": "LineString", "coordinates": list(xy)}


def polygon(x: float, y: float, size: float) -> dict[str, Any]:
    """Create an axis-aligned square."""
    return {
        "type": "Polygon",
        "coordinates": [
            [[x, y], [x + size, y], [x + size, y + size], [x, y + size], [x, y]]
        ],
    }


def additional_cases(rule: str) -> list[dict[str, Any]]:
    """Return probes whose expected identities are independent of any engine."""
    hole = polygon(0, 0, 10)
    hole["coordinates"].append(polygon(2, 2, 6)["coordinates"][0][::-1])
    cases = {
        "point/must-not-overlap": [
            (
                "duplicate triple",
                [point(0, 0)] * 3,
                3,
                {"pairs": [[1, 2], [1, 3], [2, 3]]},
            ),
        ],
        "line/must-not-have-dangles": [
            (
                "T junction with interior connection",
                [line((0, 0), (4, 0)), line((2, 0), (2, 2))],
                2,
                {"primary_ids": [1, 2]},
            ),
            (
                "closed line",
                [line((0, 0), (2, 0), (2, 2), (0, 0))],
                0,
                {"primary_ids": []},
            ),
            (
                "closed line with attached endpoint",
                [line((0, 0), (2, 0), (2, 2), (0, 0)), line((1, 0), (1, -2))],
                1,
                {"primary_ids": [2]},
            ),
        ],
        "polygon/must-not-overlap": [
            ("equal polygons", [polygon(0, 0, 4)] * 2, 1, {"pairs": [[1, 2]]}),
            (
                "contained polygon",
                [polygon(0, 0, 4), polygon(1, 1, 1)],
                1,
                {"pairs": [[1, 2]]},
            ),
            ("boundary touch", [polygon(0, 0, 4), polygon(4, 0, 4)], 0, {"pairs": []}),
            (
                "overlapping triple",
                [polygon(0, 0, 4)] * 3,
                3,
                {"pairs": [[1, 2], [1, 3], [2, 3]]},
            ),
        ],
        "polygon/must-not-have-gaps": [
            ("bounded hole", [hole], 1, {}),
            ("bounded hole with island", [hole, polygon(4, 4, 2)], 1, {}),
            ("separate exteriors", [polygon(0, 0, 2), polygon(4, 0, 2)], 0, {}),
        ],
    }
    result = [
        {
            "name": name,
            "input": geometries,
            "other": None,
            "expected": expected,
            "identity": identity,
        }
        for name, geometries, expected, identity in cases.get(rule, [])
    ]
    if rule == "polygon/must-not-have-gaps":
        bounded = polygon(2, 2, 6)
        with_island = polygon(2, 2, 6)
        with_island["coordinates"].append(polygon(4, 4, 2)["coordinates"][0][::-1])
        for case, geometries in zip(
            result, ([bounded], [with_island], []), strict=True
        ):
            case["expected_gaps"] = geometries
    return result
