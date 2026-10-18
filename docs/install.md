# Install

## CLI

Requires Rust 1.88 or newer.

```sh
cargo install topology-checker --locked
```

This build reads and writes GeoJSON, GeoPackage, Shapefile, FlatGeobuf, GeoParquet, KML and GPX without GDAL. For other formats, add GDAL:

```sh
# Build GDAL from source (needs CMake, a C/C++ compiler and sqlite3)
CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo install topology-checker --locked --features gdal
# Use an installed GDAL
cargo install topology-checker --locked --features gdal-system
```

## Python

```sh
uv add 'topology-checker[geopandas]'
```

Python 3.10 to 3.14. See [Python usage](python-usage.md).

## QGIS

Install the [CLI](#cli) (0.2.0 or newer) first; the plugin runs it and does not include it. Then install **Topology Rules** in **Plugins > Manage and Install Plugins** (QGIS 3.34 or newer; tick **Show also experimental plugins** in the Settings tab).

If `topology-checker` is not on the `PATH`, choose it in the panel, or set its path under **Settings > Options > Processing > Providers > Topology Rules**. Each rule is then a tool in the Processing toolbox, and **Check topology definition** runs a `topology.toml`.

In the **Topology Rules** panel, pick a layer, a rule, and **Add rule**; repeat, then **Run**. The errors are a tree by rule and kind, with zoom and a map filter. **Export…** and **Import…** read and write a `topology.toml`. The list is saved with the project.

## Web app

Requires Node.js and Rust 1.88 or newer.

```sh
npm ci --prefix web
npm run build --prefix web
cargo build -p topology-checker-web --release --locked
```

The frontend is embedded in `target/release/topology-checker-web`. See [Web app](web-app.md).

## RustGIS plugin

topology-checker is a plugin for [RustGIS](https://github.com/opengeos/RustGIS). Download `topology-checker-<version>.rgplugin` from the [latest release](https://github.com/alecsandrei/topology-checker/releases/latest), then in RustGIS open the Plugin Manager and choose **Install from File**. Do not unzip the file. See [RustGIS plugin](rustgis-plugin.md), which also covers building it from source.

## Rust library

```toml
[dependencies]
topology-checker-core = "0.1"
```

See the [Rust API](https://docs.rs/topology-checker-core/latest/topology_checker_core/).
