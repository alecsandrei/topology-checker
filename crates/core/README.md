# topology-checker-core

A GDAL-free, planar topology validator for `geo::Geometry<f64>` datasets. Add immutable layers, register rules, then validate. Operational failures return `Error`; violations appear in the returned `Report`.

```rust
use topology_checker_core::{geo::{line_string, Geometry}, Layer, Rule, Topology, ViolationKind};

let mut topology = Topology::new(0.001)?;
let roads = topology.add_layer(Layer::from_features("roads", [
    (41, Geometry::from(line_string![(x: 0., y: 0.), (x: 1., y: 0.)])),
    (92, Geometry::from(line_string![(x: 1., y: 0.), (x: 2., y: 0.)])),
]));
let dangles = topology.add_rule(Rule::LineMustNotHaveDangles, roads, None)?;
let report = topology.validate()?;
assert!(!report.is_valid());
assert_eq!(report.violation_count(), 2);
assert!(report.violations().all(|v| v.kind == ViolationKind::Dangle));
assert_eq!(report.rule(dangles.id).unwrap().violations[0].origin.unwrap().fid, 41);
# Ok::<(), topology_checker_core::Error>(())
```

`Layer::from_geometries` assigns sequential FIDs; `Layer::from_features` preserves supplied FIDs. `LayerId` and `RuleId` belong to the topology that created them. `add_rule` returns the registered `RuleSpec`, whose `id` finds the rule's result with `Report::rule`. Rules check input kinds when registered. Null geometries have no representation in `geo`; I/O adapters must skip them before constructing a layer. Nonfinite coordinates and duplicate FIDs are rejected. Degenerate parts and rings are skipped and reported as typed `Warning`s by `Topology::warnings()` and `validate()`; their `Display` text is stable.

Each `Violation` carries a rule, `ViolationKind`, geometry and optional origin/destination `FeatureRef`. Serialized reports keep the `errors` and `error_type` field names. A reference holds its layer, FID and optional part index. Pair violations aggregate multiple parts, clearing a part index when no single part identifies all contributing geometry. Gap violations are layer-wide and have no feature references; their `RuleReport` retains the layer context.

Part views and R-trees are cached. Polygon unions are cached per layer; line graphs are cached per participating layer set. Layer caches do not depend on a `LayerId`, so `add_layer` accepts an `Arc<Layer>` and one layer can be shared by several topologies without rebuilding its caches. Rules run in parallel and reports retain registration order. Repeated validation reuses these structures. Layers expose only immutable geometry, so the caches cannot become stale. The tolerance is explicit, finite and nonnegative, in coordinate units; the core performs no CRS lookup or reprojection.

`Rule::ALL` supplies names, input kinds, semantics and ArcGIS/QGIS equivalents. The CLI and TOML definitions use the same registry for all 38 rules.

`merge_lines(lines, tolerance)` merges original parts through eligible degree-two endpoint joins and handles isolated cycles. Interior contacts stop joins. Duplicate uses are retained; direction follows the majority of input parts, with the earliest part breaking a tie. `explode_lines(lines)` emits every nondegenerate segment independently. Both functions return `Result` and are implemented in Rust without GEOS.

Exact tolerance preserves distinct input coordinates. Proper line intersections are computed with exact rational arithmetic from their binary floating point inputs, then rounded once, so concurrent crossings share a coordinate. Polygon BooleanOps use geo's geometry engine. These are planar topology checks; polygon validity and geometry repair are outside the current API.

Binary cross-geometry rules use `Topology::add_rule(rule, layer, Some(other))`.
Public enums and report structs are `#[non_exhaustive]`, so new rules, kinds,
errors and fields are additive. The original 18 rules remain the registry prefix.
The core keeps its existing dependencies and planar tolerance model.
