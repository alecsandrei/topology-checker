# Python usage

Install from PyPI with `uv add topology-checker`, or use
`uv add 'topology-checker[geopandas]'` for GeoDataFrame exports and file loading
through GeoPandas. Python 3.10 through 3.14 on regular CPython are supported.
The base package uses Shapely and pyproj. It does not require GeoPandas or a
separate Rust installation when a compatible wheel is available.

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

`add_rule` accepts a Shapely geometry, a geometry iterable, a GeoSeries,
a GeoDataFrame, a GeoJSON mapping, or an object with `__geo_interface__`.
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

All known layer CRSs must agree. Explicit CRS metadata must agree with metadata
on the input. Unknown CRSs are accepted as caller-supplied coordinates. When
known and unknown metadata are mixed, validation continues with a warning and
the export CRS is unset. `Report.to_dict()` works without GeoPandas;
`Report.to_geodataframe()` requires the `geopandas` extra.

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

Both utilities return tuples of Shapely lines. They accept the same input
adapters and operate on planar snapshots.
