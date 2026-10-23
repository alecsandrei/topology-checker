<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/banner-dark.svg">
  <img alt="topology-checker" src="docs/images/banner-light.svg">
</picture>

<p align="center">
  <a href="https://github.com/alecsandrei/topology-checker/actions/workflows/build.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/alecsandrei/topology-checker/build.yml?branch=master&amp;label=CI"></a>
  <a href="https://github.com/alecsandrei/topology-checker/actions/workflows/build.yml"><img alt="Coverage" src="https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Falecsandrei%2Ftopology-checker%2Fbadges%2Fcoverage.json&amp;label=coverage"></a>
  <a href="https://github.com/alecsandrei/topology-checker/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/alecsandrei/topology-checker"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/github/license/alecsandrei/topology-checker"></a>
  <br>
  <a href="https://pypi.org/project/topology-checker/"><img alt="PyPI" src="https://img.shields.io/pypi/v/topology-checker"></a>
  <a href="https://pypi.org/project/topology-checker/"><img alt="Python versions" src="https://img.shields.io/pypi/pyversions/topology-checker"></a>
  <a href="https://pepy.tech/projects/topology-checker"><img alt="PyPI downloads" src="https://img.shields.io/pepy/dt/topology-checker?label=PyPI%20downloads"></a>
  <br>
  <a href="https://crates.io/crates/topology-checker"><img alt="crates.io" src="https://img.shields.io/crates/v/topology-checker"></a>
  <a href="https://crates.io/crates/topology-checker"><img alt="MSRV" src="https://img.shields.io/crates/msrv/topology-checker"></a>
  <a href="https://crates.io/crates/topology-checker"><img alt="crates.io downloads" src="https://img.shields.io/crates/d/topology-checker?label=crates.io%20downloads"></a>
</p>

<p align="center">
  <a href="https://alecsandrei.github.io/topology-checker/stable/">Documentation</a> ·
  <a href="https://alecsandrei.github.io/topology-checker/stable/install/">Install</a> ·
  <a href="https://alecsandrei.github.io/topology-checker/stable/cli/">CLI</a> ·
  <a href="https://alecsandrei.github.io/topology-checker/stable/rules/">Rules</a> ·
  <a href="https://alecsandrei.github.io/topology-checker/stable/web-app/">Web app</a> ·
  <a href="https://alecsandrei.github.io/topology-checker/stable/qgis-plugin/">QGIS plugin</a> ·
  <a href="https://alecsandrei.github.io/topology-checker/stable/rustgis-plugin/">RustGIS plugin</a> ·
  <a href="https://alecsandrei.github.io/topology-checker/stable/python-usage/">Python usage</a> ·
  <a href="https://alecsandrei.github.io/topology-checker/stable/python-api/">Python API</a> ·
  <a href="https://docs.rs/topology-checker-core/latest/topology_checker_core/">Rust API</a>
</p>

# topology-checker

topology-checker checks map data for duplicate points, gaps, overlaps, and disconnected lines. Use it with Python, the command line, the local web app, or as a [QGIS](https://alecsandrei.github.io/topology-checker/stable/qgis-plugin/) or [RustGIS](https://github.com/opengeos/RustGIS) plugin.

![Validation time for topology-checker, QGIS 4.2, GRASS GIS, PostGIS and gvSIG](benchmarks/gis/results/comparison.png)

See the [measurements](benchmarks/gis/results/README.md) and [benchmark methodology](benchmarks/gis/README.md).

## Python API

```sh
pip install "topology-checker[io]"
```

```python
from topology_checker import Rule, Topology

topology = Topology(tolerance=0.001)
topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, roads_gdf)
errors = topology.validate().to_geodataframe()
```

[More Python examples](python/README.md), the [usage guide](https://alecsandrei.github.io/topology-checker/stable/python-usage/) and the [API reference](https://alecsandrei.github.io/topology-checker/stable/python-api/).

## CLI

Requires Rust 1.88 or newer.

```sh
cargo install topology-checker --locked
```

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

`-o` writes `errors_point`, `errors_line` and `errors_polygon` layers with `violation_id`, `rule`, `error_type` and the origin and destination layer, FID and part of each violation. The web app's dataset exports use the same columns. See [CLI](https://alecsandrei.github.io/topology-checker/stable/cli/) and [Rules](https://alecsandrei.github.io/topology-checker/stable/rules/).

`topology.toml`:

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

## QGIS plugin

The Topology Rules plugin adds a Processing tool for each rule and a panel where you queue several rules, run them in one pass and browse the errors. It runs the CLI (0.2.0 or newer), so install that first, then the plugin in **Plugins > Manage and Install Plugins**. See [QGIS plugin](https://alecsandrei.github.io/topology-checker/stable/qgis-plugin/).

## Rust API

`Cargo.toml` (update the path):

```toml
[dependencies]
topology-checker-core = { path = "/path/to/topology-checker/crates/core" }
```

```rust
use topology_checker_core::{
    geo::{Geometry, line_string},
    Error, Layer, Rule, Topology, ViolationKind,
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
    assert_eq!(report.violation_count(), 2);
    assert!(report.violations().all(|v| v.kind == ViolationKind::Dangle));
    Ok(())
}
```

## Local web app

```sh
npm ci --prefix web
npm run build --prefix web
cargo run -p topology-checker-web --release --locked
```

Open <http://127.0.0.1:8080>, or pass `--port` (or set `TOPOLOGY_WEB_PORT`) to use another port. Release builds embed the frontend, so `target/release/topology-checker-web` runs on its own. Workspaces idle for four hours are discarded. See [Web app](https://alecsandrei.github.io/topology-checker/stable/web-app/).

The TypeScript API types in `web/src/generated` are generated from the server by `cargo test`.

## RustGIS plugin

topology-checker is also a [RustGIS](https://github.com/opengeos/RustGIS) plugin, with a tool for each rule. Install `topology-checker-<version>.rgplugin` from the [latest release](https://github.com/alecsandrei/topology-checker/releases/latest) with Plugin Manager › **Install from File**. See [RustGIS plugin](https://alecsandrei.github.io/topology-checker/stable/rustgis-plugin/).
