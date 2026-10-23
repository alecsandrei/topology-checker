"""WKB input and output give the same results as shapely geometries."""

from __future__ import annotations

import math
import struct
from collections.abc import Callable, Sequence
from typing import Any, cast

import pytest
import shapely
from shapely.geometry import (
    GeometryCollection,
    LineString,
    MultiLineString,
    MultiPoint,
    Point,
    box,
)
from shapely.geometry.base import BaseGeometry
from topology_checker import (
    GeometryKind,
    Report,
    Rule,
    Topology,
    Violation,
    explode_lines,
    merge_lines,
)

from .test_api import INVALID_CASES, SQUARE, VALID_BINARY_CASES

Encoder = Callable[[BaseGeometry], Any]


def iso(geometry: BaseGeometry) -> bytes:
    return shapely.to_wkb(geometry, flavor="iso", output_dimension=3)


def extended_big_endian(geometry: BaseGeometry) -> memoryview:
    data = shapely.to_wkb(
        shapely.set_srid(geometry, 3844),  # pyright: ignore[reportUnknownMemberType]
        flavor="extended",
        byte_order=0,
        output_dimension=3,
        include_srid=True,
    )
    return memoryview(data)


def iso_bytearray(geometry: BaseGeometry) -> bytearray:
    return bytearray(iso(geometry))


ENCODERS: dict[str, Encoder] = {
    "iso": iso,
    "extended-big-endian-memoryview": extended_big_endian,
    "bytearray": iso_bytearray,
}


def encode(items: Sequence[Any], encoder: Encoder = iso) -> list[Any]:
    """Encode geometries, keeping missing values as they are."""
    return [
        item if not isinstance(item, BaseGeometry) else encoder(item) for item in items
    ]


def wkb(text: str) -> bytes:
    return iso(shapely.from_wkt(text))


def summary(report: Report) -> dict[str, Any]:
    """Collect everything a caller can observe from a report."""
    return {
        "report": report.to_dict(),
        "wkb": [violation.wkb for violation in report.violations],
        "refs": [
            tuple(None if ref is None else ref.to_dict() for ref in refs)
            for refs in ((v.origin, v.destination) for v in report.violations)
        ],
        "counts": [dict(rule.counts) for rule in report.rules],
    }


def layered(
    rule: Rule, data: Sequence[Any], other: Sequence[Any] | None
) -> tuple[Topology, dict[str, Any]]:
    topology = Topology()
    primary = topology.add_layer(data, fids=range(-10, -10 + len(data)))
    target = (
        None if other is None else topology.add_layer(other, fids=range(len(other)))
    )
    topology.add_rule(rule, primary, other=target)
    return topology, summary(topology.validate())


def direct(
    rule: Rule, data: Sequence[Any], other: Sequence[Any] | None
) -> dict[str, Any]:
    topology = Topology()
    topology.add_rule(rule, data, other=other)
    with topology.iter_validate(batch_size=1) as stream:
        streamed = [violation.wkb for violation in stream]
    result = summary(topology.validate())
    assert streamed == result["wkb"]
    assert stream.summary is not None
    assert stream.summary.error_count == len(streamed)
    return result


def layers(topology: Topology) -> list[tuple[Any, ...]]:
    return [
        (layer.name, layer.fids, layer.wkb, layer.kind, layer.warnings)
        for layer in topology.layers
    ]


@pytest.mark.parametrize("encoding", ENCODERS)
@pytest.mark.parametrize(
    "canonical,rule,data,other", INVALID_CASES, ids=[case[0] for case in INVALID_CASES]
)
def test_every_rule_matches_shapely_input(
    encoding: str,
    canonical: str,
    rule: Rule,
    data: Sequence[BaseGeometry],
    other: Sequence[BaseGeometry] | None,
) -> None:
    encoder = ENCODERS[encoding]
    wkb_other = None if other is None else encode(other, encoder)
    shapely_topology, expected = layered(rule, data, other)
    wkb_topology, actual = layered(rule, encode(data, encoder), wkb_other)
    assert actual == expected
    assert expected["counts"] and sum(expected["counts"][0].values()) > 0
    assert layers(wkb_topology) == layers(shapely_topology)
    assert wkb_topology.warnings() == shapely_topology.warnings()
    assert direct(rule, encode(data, encoder), wkb_other) == direct(rule, data, other)
    assert canonical == wkb_topology.validate().rules[0].resolved_rule.value


@pytest.mark.parametrize("rule,data,other", VALID_BINARY_CASES)
def test_valid_cases_match_shapely_input(
    rule: Rule, data: BaseGeometry, other: BaseGeometry
) -> None:
    assert direct(rule, [iso(data)], [iso(other)]) == direct(rule, [data], [other])


EDGE_CASES: list[tuple[str, Rule, list[Any]]] = [
    (
        "missing-empty-and-extra-dimensions",
        Rule.MUST_NOT_OVERLAP,
        [None, Point(), Point(1, 2, 3), math.nan, Point(1, 2)],
    ),
    (
        "nested-empty-parts",
        Rule.MUST_NOT_HAVE_DANGLES,
        [
            shapely.from_wkt(
                "GEOMETRYCOLLECTION (POINT EMPTY, GEOMETRYCOLLECTION "
                "(LINESTRING EMPTY, MULTILINESTRING (EMPTY, (0 0, 1 0))))"
            )
        ],
    ),
    (
        "empty-collections",
        Rule.MUST_NOT_HAVE_DANGLES,
        [
            shapely.from_wkt(
                "GEOMETRYCOLLECTION (MULTILINESTRING EMPTY, LINESTRING (0 0, 1 0))"
            ),
            shapely.from_wkt("MULTILINESTRING ((5 5, 6 6), EMPTY, (7 7, 8 8))"),
            shapely.from_wkt("GEOMETRYCOLLECTION EMPTY"),
        ],
    ),
    (
        "z-collections",
        Rule.MUST_NOT_OVERLAP,
        [
            MultiPoint([(0, 0, 1), (1, 1, 2)]),
            Point(0, 0, 1),
            GeometryCollection([Point(), Point(1, 1, 3)]),
        ],
    ),
    (
        "collapsed-parts",
        Rule.MUST_NOT_HAVE_DANGLES,
        [MultiLineString([[(0, 0), (0, 0)], [(1, 0), (2, 0)]])],
    ),
    (
        "mixed-families",
        Rule.MUST_NOT_BE_MULTIPART,
        [
            Point(0, 0),
            shapely.from_wkt("GEOMETRYCOLLECTION (POINT (1 1), LINESTRING (0 0, 1 1))"),
        ],
    ),
]


@pytest.mark.parametrize("encoding", ENCODERS)
@pytest.mark.parametrize(
    "rule,data", [case[1:] for case in EDGE_CASES], ids=[case[0] for case in EDGE_CASES]
)
def test_normalization_matches_shapely_input(
    encoding: str, rule: Rule, data: list[Any]
) -> None:
    shapely_topology, expected = layered(rule, data, None)
    wkb_topology, actual = layered(rule, encode(data, ENCODERS[encoding]), None)
    assert actual == expected
    assert layers(wkb_topology) == layers(shapely_topology)
    assert wkb_topology.warnings() == shapely_topology.warnings()


def test_extra_dimensions_are_discarded_with_one_warning() -> None:
    point_m = b"\x01" + struct.pack("<I", 2001) + struct.pack("<3d", 1, 2, 9)
    point_zm = b"\x00" + struct.pack(">I", 3001) + struct.pack(">4d", 1, 2, 8, 9)
    extended_z = (
        b"\x01" + struct.pack("<II", 0xA0000001, 4326) + struct.pack("<3d", 1, 2, 7)
    )
    topology = Topology()
    id = topology.add_layer([point_m, point_zm, extended_z, wkb("POINT (1 2)")])
    layer = topology.layer(id)
    assert set(layer.wkb) == {wkb("POINT (1 2)")}
    assert layer.warnings == tuple(
        f"layer layer_0, FID {fid}: discarded extra dimensions" for fid in range(3)
    )
    assert all(not geometry.has_z for geometry in layer.geometries)


def test_empty_features_and_typed_hints_match_shapely_input() -> None:
    for text, kind in [
        ("LINESTRING EMPTY", GeometryKind.LINE),
        ("MULTIPOLYGON EMPTY", GeometryKind.POLYGON),
        ("POINT EMPTY", GeometryKind.POINT),
        ("GEOMETRYCOLLECTION EMPTY", None),
    ]:
        topology = Topology()
        id = topology.add_layer([wkb(text)], fids=[3])
        assert topology.layer(id).kind is kind
        assert topology.layer(id).warnings == (
            "layer layer_0, input 0, FID 3: skipped empty feature",
        )
        assert topology.layer(id).fids == ()
    with pytest.raises(ValueError, match="declared kind conflicts"):
        Topology().add_layer([wkb("POINT (0 0)")], kind=GeometryKind.LINE)
    topology = Topology()
    id = topology.add_layer(
        [wkb("GEOMETRYCOLLECTION (POINT EMPTY, LINESTRING (0 0, 1 0))")],
        kind=GeometryKind.LINE,
    )
    assert topology.layer(id).kind is GeometryKind.LINE


def test_nan_point_is_an_empty_feature() -> None:
    nan = Point(math.nan, math.nan)
    for data in ([nan], [iso(nan)]):
        topology = Topology()
        id = topology.add_layer(data)
        assert topology.layer(id).warnings == (
            "layer layer_0, input 0, FID 0: skipped empty feature",
        )


@pytest.mark.parametrize(
    "item,reason",
    [
        (b"", "unexpected end of data"),
        (b"\x05", "invalid byte order 5"),
        (b"\x01\x01\x00\x00\x00\x00", "unexpected end of data"),
        (wkb("POINT (1 2)") + b"\x00", "unexpected trailing bytes"),
        (b"\x01\x08\x00\x00\x00\x00\x00\x00\x00", "unsupported WKB geometry type 8"),
        (b"\x01\xd8\x0f\x00\x00\x00\x00\x00\x00", "unsupported WKB geometry type 4056"),
        (b"\x01\x02\x00\x00\x00\xff\xff\xff\xff", "unexpected end of data"),
        (
            b"\x01\x04\x00\x00\x00\x01\x00\x00\x00" + wkb("LINESTRING (0 0, 1 1)"),
            "MultiPoint cannot contain LineString",
        ),
        (
            b"\x01\x07\x00\x00\x00\x01\x00\x00\x00" * 200 + wkb("POINT (0 0)"),
            "geometry nesting is too deep",
        ),
    ],
)
def test_unreadable_wkb_names_layer_and_fid(item: bytes, reason: str) -> None:
    topology = Topology()
    with pytest.raises(ValueError) as raised:
        topology.add_layer([wkb("POINT (0 0)"), item], fids=[1, 7], name="roads")
    assert str(raised.value) == f"layer roads, FID 7: invalid WKB: {reason}"
    with pytest.raises(ValueError, match="layer_0, FID 0: invalid WKB"):
        topology.add_rule(Rule.MUST_NOT_OVERLAP, [item])
    assert topology.layers == ()
    assert topology.validate().rules == ()


@pytest.mark.parametrize(
    "geometry", [Point(math.inf, 0), LineString([(0, 0), (math.nan, 1)])]
)
def test_nonfinite_wkb_leaves_registration_unchanged(geometry: BaseGeometry) -> None:
    topology = Topology()
    with pytest.raises(ValueError, match="finite"):
        topology.add_layer([iso(geometry)])
    assert topology.layers == ()


@pytest.mark.parametrize("data", [b"\x01", bytearray(b"\x01"), memoryview(b"\x01")])
def test_single_wkb_value_is_not_a_layer(data: Any) -> None:
    with pytest.raises(TypeError, match="WKB items"):
        Topology().add_layer(data)


def test_snapshots_reuse_canonical_input_bytes() -> None:
    item = wkb("LINESTRING (0 0, 1 0)")
    topology = Topology()
    id = topology.add_layer([item, bytearray(item)])
    layer = topology.layer(id)
    assert layer.wkb[0] is item
    assert layer.wkb == (item, item)
    assert layer.geometries == (LineString([(0, 0), (1, 0)]),) * 2


def test_violation_exposes_iso_wkb() -> None:
    assert isinstance(Violation.wkb, property)
    topology = Topology()
    topology.add_rule(Rule.MUST_NOT_OVERLAP, [wkb("POINT (1 2)"), wkb("POINT (1 2)")])
    (violation,) = topology.validate().violations
    assert violation.wkb == struct.pack("<bIdd", 1, 1, 1.0, 2.0)
    assert violation.geometry == Point(1, 2)
    assert violation.geometry is violation.geometry


LINES: list[BaseGeometry | None] = [
    LineString([(0, 0), (1, 0)]),
    MultiLineString([[(1, 0, 5), (2, 0, 5)]]),
    None,
    shapely.from_wkt("MULTILINESTRING (EMPTY, (2 0, 2 3, 4 3))"),
]


@pytest.mark.parametrize("encoding", ENCODERS)
def test_line_utilities_return_wkb_for_wkb_input(encoding: str) -> None:
    data = encode(LINES, ENCODERS[encoding])
    for tolerance in (0.0, 0.5):
        merged = merge_lines(data, tolerance)
        assert merged == tuple(iso(line) for line in merge_lines(LINES, tolerance))
        assert all(isinstance(line, bytes) for line in merged)
    exploded = explode_lines(data)
    assert exploded == tuple(iso(line) for line in explode_lines(LINES))
    assert len(exploded) == 4
    assert merge_lines([]) == explode_lines([]) == ()


def test_line_utilities_return_shapely_for_mixed_input() -> None:
    first, last = cast(BaseGeometry, LINES[0]), LINES[3]
    mixed = [iso(first), last]
    assert merge_lines(mixed) == merge_lines([LINES[0], LINES[3]])
    assert all(isinstance(line, LineString) for line in explode_lines(mixed))
    with pytest.raises(ValueError, match="line geometries"):
        explode_lines([iso(SQUARE)])
    with pytest.raises(ValueError, match="FID 0: invalid WKB"):
        merge_lines([b"\x01"])


def test_invalid_polygon_diagnostics_keep_original_parts() -> None:
    data = shapely.from_wkt("MULTIPOLYGON (EMPTY, ((0 0, 2 2, 2 0, 0 2, 0 0)))")
    messages: list[str] = []
    for item in (data, iso(data), extended_big_endian(data)):
        with pytest.raises(ValueError) as raised:
            Topology().add_layer([item], fids=[5])
        messages.append(str(raised.value))
    assert len(set(messages)) == 1
    assert messages[0].startswith("layer layer_0, FID 5, part 1, ring")


def test_crs_metadata_applies_to_wkb_layers() -> None:
    topology = Topology()
    topology.add_layer([wkb("POINT (0 0)")], crs="EPSG:3844")
    with pytest.raises(ValueError, match="incompatible known CRSs"):
        topology.add_layer([wkb("POINT (0 0)")], crs="EPSG:4326")
    topology.add_layer([wkb("POINT (0 0)")])
    assert topology.warnings() == (
        "mixed known and unknown CRS metadata; export CRS is unset",
    )
    report = Topology()
    report.add_rule(
        Rule.MUST_BE_INSIDE, [wkb("POINT (5 5)")], other=[iso(box(0, 0, 9, 9))]
    )
    assert report.validate().is_valid


def reference_prepare(
    geometries: list[BaseGeometry | None],
) -> tuple[list[str], list[bytes], dict[int, tuple[int, ...]], GeometryKind | None]:
    """The original shapely-only normalisation, kept as an executable reference."""
    families_of = {
        "Point": GeometryKind.POINT,
        "MultiPoint": GeometryKind.POINT,
        "LineString": GeometryKind.LINE,
        "LinearRing": GeometryKind.LINE,
        "MultiLineString": GeometryKind.LINE,
        "Polygon": GeometryKind.POLYGON,
        "MultiPolygon": GeometryKind.POLYGON,
    }
    warnings: list[str] = []
    snapshots: list[bytes] = []
    parts: dict[int, tuple[int, ...]] = {}
    families: set[GeometryKind] = set()
    hints: set[GeometryKind] = set()

    def normalize(geometry: BaseGeometry) -> tuple[bool, tuple[int, ...], int]:
        positions: list[int] = []
        state = {"position": 0, "skipped": 0}

        def visit(item: BaseGeometry) -> bool:
            kind = families_of.get(item.geom_type)
            if item.is_empty and kind is not None:
                hints.add(kind)
            if (
                item.geom_type.startswith("Multi")
                or item.geom_type == "GeometryCollection"
            ):
                return any([visit(child) for child in cast(Any, item).geoms])
            original = state["position"]
            state["position"] += 1
            if item.is_empty:
                state["skipped"] += 1
                return False
            if kind is not None:
                families.add(kind)
            positions.append(original)
            return True

        return visit(geometry), tuple(positions), state["skipped"]

    for position, geometry in enumerate(geometries):
        if geometry is None or geometry.is_empty:
            if geometry is not None:
                normalize(geometry)
            label = "missing" if geometry is None else "empty"
            warnings.append(
                f"layer layer_0, input {position}, FID {position}: "
                f"skipped {label} feature"
            )
            continue
        if shapely.get_coordinate_dimension(geometry) > 2:  # pyright: ignore[reportUnknownMemberType]
            warnings.append(
                f"layer layer_0, FID {position}: discarded extra dimensions"
            )
        snapshot = shapely.to_wkb(geometry, output_dimension=2, flavor="iso")
        _, positions, skipped = normalize(shapely.from_wkb(snapshot))
        if skipped:
            warnings.append(
                f"layer layer_0, FID {position}: skipped {skipped} empty parts"
            )
        snapshots.append(snapshot)
        parts[position] = positions
    kind = (
        next(iter(families))
        if len(families) == 1
        else GeometryKind.COLLECTION
        if families
        else next(iter(hints))
        if len(hints) == 1
        else None
    )
    return warnings, snapshots, parts, kind


CORPUS = [
    "POINT (1 2)",
    "POINT Z (1 2 3)",
    "POINT EMPTY",
    "LINESTRING EMPTY",
    "LINESTRING (0 0, 1 1)",
    "LINEARRING (0 0, 1 0, 1 1, 0 0)",
    "POLYGON EMPTY",
    "POLYGON ((0 0, 1 0, 1 1, 0 0))",
    "MULTIPOINT EMPTY",
    "MULTIPOINT (EMPTY, (1 2), EMPTY, (3 4))",
    "MULTILINESTRING (EMPTY, EMPTY)",
    "MULTILINESTRING ((0 0, 1 1), EMPTY, (2 2, 3 3))",
    "MULTIPOLYGON (EMPTY, ((0 0, 1 0, 1 1, 0 0)))",
    "GEOMETRYCOLLECTION EMPTY",
    "GEOMETRYCOLLECTION (POINT EMPTY, LINESTRING EMPTY)",
    "GEOMETRYCOLLECTION (GEOMETRYCOLLECTION EMPTY, POINT (1 1))",
    "GEOMETRYCOLLECTION (POINT (0 0), MULTIPOINT (EMPTY, (1 1)), "
    "GEOMETRYCOLLECTION (LINESTRING EMPTY, POLYGON ((0 0, 1 0, 1 1, 0 0))))",
    "GEOMETRYCOLLECTION (MULTIPOLYGON EMPTY, MULTILINESTRING EMPTY)",
]


@pytest.mark.parametrize("size", [1, 2, 3])
def test_normalization_matches_original_shapely_rules(size: int) -> None:
    geometries = [shapely.from_wkt(text) for text in CORPUS]
    for start in range(len(geometries)):
        data: list[BaseGeometry | None] = [*geometries[start : start + size], None]
        warnings, snapshots, parts, kind = reference_prepare(data)
        for items in (data, encode(data), encode(data, extended_big_endian)):
            topology = Topology()
            layer = topology.layer(topology.add_layer(items))
            assert layer.warnings == tuple(warnings)
            assert layer.wkb == tuple(snapshots)
            assert layer.kind is kind
            assert layer.fids == tuple(parts)
            for fid, positions in parts.items():
                assert [
                    layer.reference(fid, i).part for i in range(len(positions))
                ] == list(positions)
