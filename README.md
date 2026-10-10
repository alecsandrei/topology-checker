# topology-checker

## Installation

Prerequisites: Rust **1.88 or newer**, CMake, a C/C++ compiler and `sqlite3` on PATH.
On macOS, install Xcode Command Line Tools; on Linux, install the compiler
toolchain, CMake and SQLite; on Windows, install Visual Studio Build Tools,
CMake and SQLite.

From the repository root:

```sh
cargo install --path crates/cli --locked
```

The CLI bundles GDAL and PROJ by default. The first build takes several minutes.
To use an installed GDAL instead:

```sh
cargo install --path crates/cli --no-default-features --locked
```

To build a local release binary:

```sh
cargo build --workspace --release --locked
./target/release/topology-checker --help
```

The Rust core builds independently without GDAL, PROJ, CMake or SQLite:

```sh
cargo build -p topology-checker-core --release --locked
```

## CLI examples

```sh
topology-checker rules
topology-checker --json rules
topology-checker drivers --read --write

topology-checker line must-not-have-dangles roads.gpkg --layer roads -o errors.gpkg
topology-checker line must-not-overlap-with roads.gpkg --other rivers.shp -o overlaps.gpkg
topology-checker polygon must-not-have-gaps parcels.geojson --tolerance 0.001 -o gaps.gpkg
topology-checker --json point must-not-overlap points.geojson
topology-checker point must-be-covered-by-lines stops.geojson --other routes.gpkg
topology-checker polygon must-contain-one-point parcels.gpkg --other addresses.gpkg
topology-checker line must-not-intersect-polygons roads.gpkg --other buildings.gpkg
topology-checker --overwrite --driver GPKG line must-not-have-dangles roads.gpkg -o errors.gpkg

topology-checker utilities merge-lines roads.gpkg --layer roads -o merged.gpkg
topology-checker utilities explode-lines roads.gpkg --layer roads -o segments.gpkg
topology-checker line must-not-have-dangles --help
```

The `rules` command groups checks by geometry type and shows their required inputs:

```text
Available topology rules (38)

Points (10)

  point/must-not-overlap
    Find duplicate points within the distance tolerance.
    Needs: one point layer

  point/must-not-overlap-with
    Find duplicate points across two layers within the distance tolerance.
    Needs: two point layers

...
```

See [Cross-geometry topology checks](docs/cross-geometry-checks.md) for ArcGIS and
QGIS support, input types, boundary semantics, acceptance-test traceability, and
this checker's coverage of mixed-geometry relationships. The 38 rules are grouped
as Points (10), Lines (19), Polygons (7), and All geometries (2).

Save a topology definition as `topology.toml`:

```toml
tolerance = 0.001

[layers]
roads = { path = "city.gpkg", layer = "roads" }
rivers = { path = "rivers.shp" }
parcels = { path = "city.gpkg", layer = "parcels" }
addresses = { path = "addresses.gpkg" }

[[rules]]
rule = "line/must-not-have-dangles"
layer = "roads"

[[rules]]
rule = "line/must-not-overlap-with"
layer = "roads"
other = "rivers"

[[rules]]
rule = "polygon/must-not-have-gaps"
layer = "parcels"

[[rules]]
rule = "polygon/must-contain-one-point"
layer = "parcels"
other = "addresses"
```

```sh
topology-checker --json validate topology.toml -o errors.gpkg
```

All binary rules use the same `--other` CLI option and TOML `layer`/`other`
fields. Distances are planar coordinate units. Polygon boundary checks retain
individual exterior, hole and shared parcel edges. Required line containment uses
the polygon layer union. Point cardinality counts feature IDs, with all retained
parts of a point feature required to lie strictly inside a polygon feature.

The new public `Rule` and `ErrorType` variants are additive; downstream exhaustive
Rust matches need updating. Rule catalog and report JSON fields are unchanged.

## Rust API example

Add the core to your application's `Cargo.toml`, using your checkout path:

```toml
[dependencies]
topology-checker-core = { path = "/path/to/topology-checker/crates/core" }
```

```rust
use topology_checker_core::{
    geo::{Geometry, line_string},
    Error, ErrorType, Layer, Rule, Topology,
};

fn main() -> Result<(), Error> {
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
    Ok(())
}
```
