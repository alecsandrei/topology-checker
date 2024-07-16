A small project, **currently under development**, that can be used to check the topology of vector datasets. It's goal is to be faster than current alternatives. It has a CLI.

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
