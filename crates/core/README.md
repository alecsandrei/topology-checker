# topology-checker-core

A GDAL-free, planar topology validator for `geo::Geometry<f64>` datasets. Add immutable layers, register rules, then validate. Operational failures return `Error`; violations appear in the returned `Report`.

```rust
use topology_checker_core::{geo::{line_string, Geometry}, ErrorType, Layer, Rule, Topology};

let mut topology = Topology::new(0.001)?;
let roads = topology.add_layer(Layer::from_features("roads", [
    (41, Geometry::from(line_string![(x: 0., y: 0.), (x: 1., y: 0.)])),
    (92, Geometry::from(line_string![(x: 1., y: 0.), (x: 2., y: 0.)])),
]));
topology.add_rule(Rule::LineMustNotHaveDangles, roads, None)?;
let report = topology.validate()?;
assert!(!report.is_valid());
assert_eq!(report.error_count(), 2);
assert!(report.errors().all(|error| error.error_type == ErrorType::Dangle));
assert_eq!(report.errors().next().unwrap().origin.unwrap().fid, 41);
# Ok::<(), topology_checker_core::Error>(())
```

`Layer::from_geometries` assigns sequential FIDs; `Layer::from_features` preserves supplied FIDs. `LayerId` belongs to the topology that created it. Rules check input kinds when registered. Null geometries have no representation in `geo`; I/O adapters must skip them before constructing a layer. Nonfinite coordinates and duplicate FIDs are rejected. Degenerate parts and rings are skipped and reported by `Topology::warnings()` and `validate()`.

Each error carries a rule, error type, geometry and optional origin/destination `FeatureRef`. A reference holds its layer, FID and optional part index. Pair errors aggregate multiple parts, clearing a part index when no single part identifies all contributing geometry. Gap errors are layer-wide and have no feature references; their `RuleReport` retains the layer context.

Part views and R-trees are cached. Polygon unions are cached per layer; line graphs are cached per participating layer set. Rules run in parallel and reports retain registration order. Repeated validation reuses these structures. Layers expose only immutable geometry, so the caches cannot become stale. The tolerance is explicit, finite and nonnegative, in coordinate units; the core performs no CRS lookup or reprojection.

`Rule::ALL` supplies names, input kinds, semantics and ArcGIS/QGIS equivalents. The CLI and TOML definitions use the same registry. See the repository's `docs/overview.md` for all 38 rules.

`merge_lines(lines, tolerance)` merges original parts through eligible degree-two endpoint joins and handles isolated cycles. Interior contacts stop joins. Duplicate uses are retained; direction follows the majority of input parts, with the earliest part breaking a tie. `explode_lines(lines)` emits every nondegenerate segment independently. Both functions return `Result` and are implemented in Rust without GEOS.

Exact tolerance preserves distinct input coordinates. Proper line intersections are computed with exact rational arithmetic from their binary floating point inputs, then rounded once, so concurrent crossings share a coordinate. Polygon BooleanOps use geo's geometry engine. These are planar topology checks; polygon validity and geometry repair are outside the current API.

Binary cross-geometry rules use `Topology::add_rule(rule, layer, Some(other))`.
New public enum variants require downstream exhaustive `Rule` and `ErrorType`
matches to be updated. The original 18 rules remain the registry prefix.
The core keeps its existing dependencies and planar tolerance model.
