A small project, **currently under development**, that can be used to check the topology of vector datasets. It's goal is to be faster than current alternatives. It has a CLI.

See [docs/overview.md](docs/overview.md) for how it works, how its rules map to ArcGIS and QGIS topology rules, known problems and the roadmap.

# Building

GDAL and PROJ do **not** need to be installed. They are compiled from source and linked statically, so the resulting binary (`target/release/topology-checker`) is self-contained and can be copied to other machines with the same OS.

Prerequisites:
- [Rust](https://rustup.rs) 1.88 or newer
- CMake
- A C/C++ compiler (Xcode Command Line Tools on macOS, `build-essential` on Linux, Visual Studio Build Tools on Windows)
- The `sqlite3` command line tool, used by the PROJ build. It ships with macOS. On Linux run `sudo apt-get install sqlite3`. On Windows run `winget install SQLite.SQLite` or `choco install sqlite`.

```sh
cargo build --release
```

The first build compiles GDAL and PROJ and takes several minutes. Later builds reuse them.

The bundled GDAL includes these drivers: GPKG, ESRI Shapefile, GeoJSON, FlatGeobuf, CSV, OpenFileGDB, MapInfo File, VRT and Memory. To link against a GDAL already installed on the system (found through `pkg-config`) instead, build with `cargo build --release --no-default-features`.

# Basic usage

## Listing accepted drivers
```sh
topology-checker gdal-drivers write
topology-checker gdal-drivers read
topology-checker gdal-drivers read-and-write
```

## Performing checks
```sh
topology-checker line must-not-have-dangles LINES="./assets/lines.shp" DANGLES="./assets/dangles.shp"

topology-checker line must-not-intersect LINES="./assets/lines.shp" SINGLE_POINTS="./assets/point_intersections.shp" COLLINEAR_LINES="./assets/line_intersections.shp"

topology-checker polygon must-not-overlap GEOMETRY="./assets/buildings.shp" OVERLAPS="./assets/overlaps.shp"

topology-checker point must-not-overlap-with POINTS="./assets/buildings.shp" OVERLAPS="./assets/overlaps.shp" OTHER="./assets/buildings_other.shp"
```

# As a library
It depends on the [geo](https://crates.io/crates/geo) crate. Import the following traits:
```rust
use topology_checker::rule::{MustNotHaveDangles, MustNotIntersect, MustNotOverlap};
```
equivalent with
```rust
use topology_checker::prelude::*;
```
These rules will now be implemented for vectors of certain geometries. For example, you could do:
```rust
use geo::{line_string, Line, LineString, Point};
use topology_checker::rule::MustNotHaveDangles;
let lines: Vec<LineString> = vec![
    line_string![
        (x: 0., y: 0.),
        (x: 1., y: 1.)],
    line_string![
        (x: 1., y: 1.),
        (x: 2., y: 2.)]];
let dangles: Vec<Point> = lines.must_not_have_dangles();

use topology_checker::rule::MustNotIntersect;
let intersections: (Vec<geo::Line>, Vec<geo::Point>) = lines.must_not_intersect();

use topology_checker::rule::MustNotOverlap;
use geo::{polygon, Polygon};
let polygons: Vec<Polygon> = vec![
    polygon![
        (x: 0., y: 0.),
        (x: 1., y: 0.),
        (x: 1., y: 1.),
        (x: 0., y: 1.),
        (x: 0., y: 0.)],
    polygon![
        (x: 1., y: 1.),
        (x: 2., y: 1.),
        (x: 2., y: 2.),
        (x: 1., y: 2.),
        (x: 1., y: 1.)]];
let overlaps: Vec<Polygon> = polygons.clone().must_not_overlap();
let others: Vec<Polygon> = vec![
    polygon![
        (x: 0., y: 0.),
        (x: 1., y: 0.),
        (x: 1., y: 1.),
        (x: 0., y: 1.),
        (x: 0., y: 0.)],
    polygon![
        (x: 1., y: 1.),
        (x: 2., y: 1.),
        (x: 2., y: 2.),
        (x: 1., y: 2.),
        (x: 1., y: 1.)]];
let overlaps: Vec<Polygon> = polygons.clone().must_not_overlap_with(others);
```
