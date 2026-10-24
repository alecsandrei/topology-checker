# Python usage

Install with `pip install "topology-checker[io]"` for Shapely geometries,
GeoPandas input and export, and CRS metadata. `pip install topology-checker`
has no dependencies and works with [WKB](#wkb-input-and-output) only. Python
3.10 through 3.14 on regular CPython are supported. Wheels need no Rust
installation. See the [API reference](python-api.md) for every class and
function.

## Register and validate

```python
import geopandas as gpd
from topology_checker import Rule, Topology

roads_gdf = gpd.read_file("roads.gpkg")
topology = Topology(tolerance=0.001)
topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, roads_gdf)
report = topology.validate()
errors = report.to_geodataframe()
print(report.is_valid, report.error_count, report.warnings)
```

`add_rule` accepts a Shapely geometry, a sequence of geometries or WKB items,
a GeoSeries, a GeoDataFrame, a GeoJSON mapping, or an object with
`__geo_interface__`.
Pass the second layer with the keyword argument `other=`. Use enum members
such as `Rule.MUST_NOT_OVERLAP`; parse serialized names with
`Rule("must-not-overlap")` before registration.

Each raw input creates a fresh XY snapshot at registration. Later edits to a
GeoDataFrame or list do not change that layer. Passing the same raw object to
two rules creates two layers, and a generator is consumed on each call.
`validate()` returns an immutable report snapshot; topology violations are
results, while invalid registrations raise exceptions and leave the topology
unchanged.

## Reuse a layer explicitly

```python
from shapely.geometry import LineString
from topology_checker import Rule, Topology

topology = Topology(tolerance=0.001)
roads = topology.add_layer(
    [LineString([(0, 0), (1, 0)]), LineString([(1, 0), (2, 0)])],
    name="roads",
    fids=[41, 92],
    crs="EPSG:28992",
)
topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, roads)
topology.add_rule(Rule.MUST_NOT_OVERLAP, roads)
report = topology.validate()
print(topology.layer(roads).name)
print(report.to_dict())
```

The returned `LayerId` belongs to its originating `Topology`. Reuse it across
rules to share the registered layer and native caches. Binary rules require
distinct IDs. A `RuleSpec` returned by `add_rule` also exposes the registered
IDs through `.layer` and `.other`.

FIDs default to original input positions, independent of a GeoDataFrame's
index or GeoJSON feature IDs. Explicit FIDs must be unique signed 64-bit
integers, one per original position, including positions subsequently skipped.
Reports preserve FID and multipart provenance in `.origin` and `.destination`.
Missing and empty features are skipped with warnings, extra dimensions are
discarded, and nonfinite retained XY coordinates are rejected. Mixed nonempty
families are accepted only by rules with an `ANY` input signature.

## WKB input and output

```python
import struct
from topology_checker import Rule, Topology

def line(*coords):
    header = struct.pack("<BII", 1, 2, len(coords))
    return header + b"".join(struct.pack("<dd", x, y) for x, y in coords)

topology = Topology(tolerance=0.001)
roads = topology.add_layer([line((0, 0), (1, 0)), line((1, 0), (2, 0))], fids=[41, 92])
topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, roads)
for violation in topology.validate().violations:
    print(violation.origin, violation.wkb.hex())
```

WKB items are `bytes`, `bytearray` or `memoryview` in ISO or extended WKB,
either byte order; `None` is a missing feature. Unreadable WKB raises
`ValueError` naming the layer and FID. `Violation.wkb` and `Layer.wkb` are 2D
little-endian ISO WKB. `Violation.geometry` and `Layer.geometries` decode them
with Shapely.

## Declare ambiguous empty layers

```python
from topology_checker import GeometryKind, Rule, Topology

topology = Topology()
empty_lines = topology.add_layer([], kind=GeometryKind.LINE)
topology.add_rule(Rule.MUST_NOT_OVERLAP, empty_lines)
assert topology.validate().is_valid
```

Family inference uses nonempty parts first, then empty geometry hints. An empty
list, only missing values, or conflicting empty hints have no inferred family.
Rules with multiple possible signatures then require a declaration through
`add_layer(..., kind=GeometryKind.POINT)`, `LINE`, or `POLYGON`. A declaration
must agree with every retained nonempty part. A uniquely matching rule, such as
`MUST_NOT_HAVE_DANGLES`, can resolve even with unknown empty input.

## CRS and tolerance

The engine operates in a Cartesian XY plane. It does not reproject, calculate
geodesic distances, or infer a CRS from coordinates. Reproject geographic data
to an appropriate projected CRS before using distance tolerances.

Tolerance is finite, nonnegative, and expressed in the inputs' coordinate units.
For a CRS using metres, `0.001` is one millimetre. It is interpreted by each rule,
such as a distance threshold, a minimum retained line length, or a polygon sliver
filter. It does not apply a universal geometry buffer or repair geometries.
Inspect the concrete rule description for the relevant behavior.

Within the tolerance, coordinates are treated as coincident, so a near miss
within tolerance is a touch, not a dangle or an overlap. Set the tolerance to the
data's positional precision. Tolerance 0 is exact on the binary coordinates, so
floating-point slivers become reportable.

All known layer CRSs must agree. Explicit CRS metadata must agree with metadata
on the input. Unknown CRSs are accepted as caller-supplied coordinates. When
known and unknown metadata are mixed, validation continues with a warning and
the export CRS is unset. `crs=`, `to_dict()` with violations and
`Report.to_geodataframe()` require the `io` extra.

## Geometry-dependent semantics

The 24 public constraints resolve to the 38 concrete [rules](rules.md) using the ordered
geometry families and whether `other=` is supplied. These examples illustrate
why the concrete variant matters:

| Public rule and inputs | Meaning |
| --- | --- |
| `MUST_NOT_OVERLAP`, points | Coincident point pairs within tolerance. |
| `MUST_NOT_OVERLAP`, lines | Shared linework between distinct features; crossings and self-overlaps are excluded. |
| `MUST_NOT_OVERLAP`, polygons | Shared areas between distinct features, excluding slivers below tolerance. |
| `MUST_NOT_OVERLAP`, lines then polygons | Interior or shared boundary line pieces longer than tolerance; isolated contacts are allowed. |
| `MUST_BE_INSIDE`, points then polygons | Strict interior membership; boundary proximity within tolerance is outside. |
| `MUST_BE_INSIDE`, lines then polygons | Pieces outside the polygon union longer than tolerance; boundary linework is covered. |
| `MUST_NOT_INTERSECT`, lines | Overlaps and interior crossings between distinct features; endpoint touches are allowed. |
| `MUST_NOT_INTERSECT`, lines then polygons | Interior pieces, shared boundary pieces and isolated contacts, including boundary proximity within tolerance. |
| `MUST_NOT_HAVE_GAPS`, polygons | Enclosed gaps and an outer-boundary diagnostic; inspect error categories. |
| `MUST_NOT_BE_CONTAINED_BY`, any then any | Whole cleaned features contained by individual targets, excluding mutual containment; no tolerance buffer. |

Order matters: line then polygon signatures do not imply polygon then line
support. Multipart parts of one feature retain their FID; use the dedicated
self-intersection or self-overlap constraints to examine a feature internally.

```python
from shapely.geometry import Point, box
from topology_checker import Rule, Topology

topology = Topology()
topology.add_rule(Rule.MUST_BE_INSIDE, [Point(0.5, 0.5)], other=[box(0, 0, 1, 1)])
result = topology.validate().rules[0]
print(result.resolved_rule.value)
print(result.resolved_rule.description)
for variant in Rule.MUST_NOT_OVERLAP.variants:
    print(variant.inputs, variant.description)
```

Use `.resolved_rule` on a `RuleSpec`, `RuleReport`, or `Violation` to inspect
the selected semantics. Invalid arity, unsupported families, or ambiguous
empty inputs are reported at registration with the supported signatures.

## Line utilities

```python
from shapely.geometry import LineString
from topology_checker import explode_lines, merge_lines

lines = [LineString([(0, 0), (1, 0)]), LineString([(1, 0), (2, 0)])]
merged = merge_lines(lines, tolerance=0.001)
segments = explode_lines(merged)
assert len(segments) == 2
```

Both utilities accept the same inputs as `add_layer` and operate on planar
snapshots. They return tuples of Shapely lines, or of WKB bytes when every
input is WKB.

## Input validity and tolerance

Polygon registration rejects self-intersections, closing spikes, crossing or
nested holes, and overlapping, nested, or edge-sharing parts of a MultiPolygon.
Empty members and degenerate rings are cleaned first. Errors identify the layer,
original FID, part, ring, and reason. Registration is atomic. Repair data explicitly
before registration if needed; the validator preserves original coordinates.

A line part whose original vertices all lie within the tolerance of each other is
collapsed. It contributes no edges to line graphs built with that tolerance and
receives one warning identifying its layer, FID, and original part. Warnings
appear in `warnings()` and reports.
Closed parts have no endpoints. In the line `MUST_NOT_INTERSECT` and
`MUST_NOT_INTERSECT_OR_TOUCH_INTERIOR` rules, a feature's endpoints are where an odd
number of its open parts end (OGC mod-2); dangle, pseudo-node and endpoint rules use
every open part end. A polygon-boundary crossing within tolerance of
an open part endpoint is exempt from `MUST_NOT_CROSS_BOUNDARIES`; closed parts have
no exemption. Boundary intervals pass `MUST_BE_INSIDE`, fail
`MUST_BE_PROPERLY_INSIDE`, and are reported as lines by intersection rules.

Polygon overlaps and gaps pass the sliver test when their positive area and
perimeter satisfy `2 * area / perimeter >= tolerance`. Perimeter includes hole
boundaries. Zero tolerance retains positive-area decisions. CRS compatibility
ignores axis order, so EPSG:4326 and OGC:CRS84 can share a topology. Projection,
datum, and unit differences still require reprojection.

## Stream validation

```python
with topology.iter_validate(batch_size=1024) as stream:
    for violation in stream:
        print(violation.error_type, violation.origin)
    summary = stream.summary
    print(summary.error_count, summary.warnings)
```

The stream captures immutable layer and rule snapshots. It yields aggregated
violations in rule, category, and feature order, preserving every duplicate pair.
Use the context manager or `close()` when stopping early. Closing cancels native
work and releases queued geometry. `summary` is available after successful
exhaustion and stays `None` after cancellation or failure. `validate()` and its
`Report` remain eager and compatible. Large duplicate stacks still produce a
quadratic number of pairs, so stream them when the complete report is too large
for memory.

```python
with topology.iter_reports() as reports:
    for rule in reports:
        print(rule.resolved_rule.value, len(rule.violations))
```

`iter_reports()` yields each rule's `RuleReport` as soon as that rule finishes,
in registration order, and otherwise behaves like `iter_validate()`.

Polygon area and boundary decisions retain exact node identities until output.
The inclusive area-to-perimeter threshold is certified before geometry emission,
with exact rational calculations for cancellation and ambiguous thresholds.
Original vertices stay fixed. Intersection nodes share one output coordinate;
a local search preserves their topology within at most 16 floating-point steps
and 100,000 candidate assignments per affected neighborhood.

An unsatisfied output constraint raises `topology_checker.PrecisionError`, a
`RuntimeError` subclass. Its message identifies the operation, rule, and available
layer and FID context. Eager validation returns no partial report. A failed stream
raises the same exception and leaves `summary` as `None`.
