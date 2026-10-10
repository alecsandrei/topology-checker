# topology-checker: architecture, rules and migration

The `redesign` branch replaces the `294ea80` implementation at version `0.1.0-alpha`. It implements 18 rules in a GDAL-free core and a separate GDAL CLI. The [README](../README.md) describes building and commands; [performance notes](performance.md) document measurements.

## Architecture

```text
GDAL reader: all physical layers or named selectors, FIDs, CRS, XY coordinates
    -> immutable core Layers
    -> cached single-part views and R-trees
    -> shared line graphs or polygon unions
    -> registered rules in parallel
    -> Report with typed errors and feature provenance
    -> human/JSON summary, optional GDAL error datasets
```

| Path | Responsibility |
|---|---|
| [`crates/core/src/model.rs`](../crates/core/src/model.rs) | Layers, feature references, single-part caches, kind validation |
| [`crates/core/src/rule.rs`](../crates/core/src/rule.rs) | Registry used by the library, CLI, TOML and rule listing |
| [`crates/core/src/topology.rs`](../crates/core/src/topology.rs) | Rule registration, scoped graph caches, validation and reports |
| [`crates/core/src/engine`](../crates/core/src/engine) | Indexed candidate joins, endpoint clustering, noding and polygon unions |
| [`crates/core/src/rules`](../crates/core/src/rules) | Point, line, polygon and multipart checks |
| [`crates/core/src/merge.rs`](../crates/core/src/merge.rs) | Independent Rust chain/cycle merger and segment explosion |
| [`crates/cli/src`](../crates/cli/src) | Argument/config parsing, GDAL I/O and summaries |

The core uses geo 0.33, rstar, rayon, serde and thiserror. Exact rational arithmetic stabilizes proper line intersections, preventing distinct pairwise roundings of one concurrent crossing from creating a false edge. The CLI uses GDAL 0.19 and bundles GDAL/PROJ by default. There is no GEOS or GDAL dependency in the core.

Features retain their layer and signed 64-bit FID. Single-part views retain stable part indices. Pair errors aggregate by feature identity and error type, using multipart geometry and an absent part index where necessary. Geometry never becomes an identity key. Duplicate line uses remain in the graph.

Line graphs are built separately for each participating layer set, so unrelated layers cannot change a rule's results. R-tree segment candidates are noded at crossings and overlap endpoints. A positive tolerance cracks nearby endpoints onto segments, then clusters nodes using a deterministic grid search with stable representatives. Input geometry remains immutable. Edge uses and node incidence drive the line rules. Proper intersections share one consistently rounded coordinate even at zero tolerance.

Polygon overlaps use intersection areas. Gaps use interior and exterior rings of the layer union. Line containment clips against the polygon union. Polygon windings are normalized before union, and overlap pieces and hole rings narrower than tolerance (`2 * area / perimeter`) are removed.

## Rules

Names follow [ArcGIS polygon rules](https://doc.esri.com/en/arcgis-pro/latest/help/editing/geodatabase-topology-rules-for-polygon-features.html), [ArcGIS line rules](https://doc.esri.com/en/arcgis-pro/latest/help/editing/geodatabase-topology-rules-for-polyline-features.html) and relevant [QGIS Topology Checker rules](https://docs.qgis.org/latest/en/docs/user_manual/plugins/core_plugins/plugins_topology_checker.html). The registry exposes the exact equivalents. These implementations do not provide ArcGIS's full validation, tolerance repair, exceptions or fix workflow.

| Rule | Reports | Geometry and error type |
|---|---|---|
| `point/must-not-overlap` | Distinct feature pairs within tolerance | Point(s), `overlap` |
| `point/must-not-overlap-with` | Point pairs across two layers within tolerance | Point(s), `overlap` |
| `point/must-be-inside` | Points not strictly inside any polygon, including boundary distances at or below tolerance | Point, `outside` |
| `line/must-not-have-dangles` | Open part endpoints with exactly one incident use | Point, `dangle` |
| `line/must-not-have-pseudo-nodes` | Degree-two nodes joining endpoints of two distinct features | Point, `pseudo-node` |
| `line/must-not-overlap` | Shared linework of distinct features | Line(s), `overlap` |
| `line/must-not-overlap-with` | Shared linework across two layers | Line(s), `overlap` |
| `line/must-not-self-overlap` | Repeated linework within one feature, including multiple parts | Line(s), `self-overlap` |
| `line/must-not-intersect` | Overlaps and interior-to-interior meetings of distinct features | Line(s), `overlap`; Point, `crossing` |
| `line/must-not-intersect-with` | The same across two layers | Line(s), `overlap`; Point, `crossing` |
| `line/must-not-intersect-or-touch-interior` | Overlaps and meetings where at least one feature is interior | Line(s), `overlap`; Point, `crossing` or `touch-interior` |
| `line/must-not-intersect-or-touch-interior-with` | The same across two layers | As above |
| `line/must-not-self-intersect` | Self-overlap and true self-crossing, allowing closed loops and self-touch | Line(s), `self-overlap`; Point, `self-crossing` |
| `line/must-be-inside` | Uncovered pieces outside the polygon layer union, longer than tolerance | Line(s), `outside` |
| `polygon/must-not-overlap` | Intersection areas of distinct feature pairs, excluding slivers | Polygon(s), `overlap` |
| `polygon/must-not-overlap-with` | Intersection areas across two layers, excluding slivers | Polygon(s), `overlap` |
| `polygon/must-not-have-gaps` | Union holes excluding slivers, plus exterior rings | Line, `gap` or `outer-boundary` |
| `geometry/must-not-be-multipart` | Features with more than one part | Original geometry, `multipart` |

A one-part Multi geometry is valid for the multipart rule. Pair overlaps are reported once per feature pair/type; crossing locations remain separate errors. Closed loops do not produce dangles or pseudo nodes. Multipart line interior classification uses endpoint parity. Self-crossings require alternating passages through a node, rather than any repeated visit.

`utilities merge-lines` follows degree-two endpoint chains and then isolated cycles, using the strategy documented by [GEOS LineMerger](https://libgeos.org/doxygen/classgeos_1_1operation_1_1linemerge_1_1LineMerger.html). It preserves original parts at interior junctions. Traversal is linear after indexed contact discovery; index building and queries have separate costs. It is implemented independently in Rust, with no copied GEOS source or GEOS dependency. `utilities explode-lines` writes every nondegenerate segment, preserving repeated uses.

## Fixed historical problems

The previous overview reproduced these failures against `294ea80`. The cases are retained below as migration context and are covered by acceptance or CLI tests.

| Previous failure | Result after redesign |
|---|---|
| Clean checks with output paths panicked | Exit 0 and no output for clean data |
| Interactive export panicked when errors were found | Interactive mode replaced by TOML validation; mixed output writes tested |
| Wrong geometry kinds panicked | Typed error with layer, FID, expected and found kind; CLI exit 2 |
| Empty or zero-length lines panicked | Empty/degenerate parts skipped with warnings |
| Null feature geometry aborted GDAL reading | Skipped and counted consistently |
| Unknown output extensions panicked | Explicit driver diagnostic and exit 2 |
| Identical lines `(0 0, 2 0)` lost their overlap | One overlap per distinct feature pair, including reversed duplicates |
| Partial overlap `(0 0, 2 0)` against `(1 0, 3 0)` was missed | Shared piece `(1 0, 2 0)` reported |
| Partial self-overlap `(0 0, 2 0, 2 1, 1 1, 1 0, 3 0)` was missed | Shared piece reported by the self-overlap rule |
| Adjacent squares with an extra shared vertex produced false gaps | Union produces one outer boundary and no hole |
| One-part MultiPolygon was classified as multipart | Valid, with zero errors |
| Endpoints 1e-9 apart produced four dangles | At tolerance 0.001 they connect, leaving two dangles; at zero they remain distinct |
| Self-crossings appeared in distinct-feature rules | Separate self-intersection and self-overlap rules |
| Line containment checked individual polygons | Coverage by the whole polygon layer union, with only outside pieces returned |
| Feature IDs were discarded | Preserved through reading, part views, errors and output |
| Only the first dataset layer was read | All layers checked separately, or explicit selectors |
| `KEY=value` keys were ignored | Positional input plus named options; legacy syntax removed |
| Output driver matching depended on substrings and iteration order | Exact extension tokens, explicit ambiguity errors; 12 GeoJSON runs tested |
| Explicit driver was ignored by rule exports | Honored by all outputs |
| Existing outputs were replaced implicitly | Replacement requires an explicit overwrite option |
| Core users had to compile GDAL | Core and CLI separated |
| Optimized line merger failed three of its tests | Both original suites ported, all 14 pass |
| Visited-pair and inside-point searches were quadratic | Indexed pair generation and parallel predicates, with no visited-pair vector search |
| README library example was obsolete | Core README compiled and executed as a doctest |

The historical timings were 3.77 seconds for 160k line dangles, 6.45 seconds for 80k polygon overlaps, and 0.65 seconds for 160k point containment on an Apple M5. They did not preserve exact workload definitions. Current benchmarks and direct old/new comparisons use documented generated inputs; see [performance notes](performance.md).

## Verification and remaining work

Acceptance tests cover overlaps, boundaries, slivers, tolerance, multipart identity, graph isolation, closed loops, self-touch and concurrent crossing precision. Property tests compare random integer-grid datasets with brute-force DE-9IM, distance and containment references. CLI tests cover exit codes, overwrite behavior, exact driver selection, warnings, selectors, Cartesian layer pairs, CRS defaults and mismatch, TOML, utilities, nullable identity and mixed error families. CI configures release builds, tests, formatting, Clippy and artifacts on Ubuntu, Windows and macOS, plus Rust 1.88.

Invalid and duplicate geometry rules, exceptions, fixes, tiling, incremental validation, reprojection, the covered-by family and contains-point remain out of scope. The current error identity supports future exceptions and fixes. Dense datasets can generate quadratically many genuine intersections or error pairs; indexed candidate discovery cannot avoid the output cost. Geometry and graph structures remain in memory. Overwriting an output is not an atomic replacement of the old dataset.
