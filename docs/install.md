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

## Web app

Requires Node.js and Rust 1.88 or newer.

```sh
npm ci --prefix web
npm run build --prefix web
cargo build -p topology-checker-web --release --locked
```

The frontend is embedded in `target/release/topology-checker-web`. See [Web app](web-app.md).

## Rust library

```toml
[dependencies]
topology-checker-core = "0.1"
```

See the [Rust API](https://alecsandrei.github.io/topology-checker/rust/topology_checker_core/).
