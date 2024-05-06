A small project, currently under development, that can be used to check the topology of vector datasets. It has a CLI.

# Basic usage

```sh
topology-checker must-not-have-dangles LINES='./assets/lines.shp' DANGLES='./assets/dangles.shp'

topology-checker must-not-intersect LINES='./assets/lines.shp' DANGLES='./assets/dangles.shp'

topology-checker must-not-intersect LINES='./assets/lines.shp' SINGLE_POINTS='./assets/point_intersections.shp' COLLINEAR_LINES='./assets/line_intersections.shp'

topology-checker must-not-overlap GEOMETRY='./assets/buildings.shp' OVERLAPS='./assets/overlaps.shp'

topology-checker must-not-overlap GEOMETRY='./assets/buildings.shp' OVERLAPS='./assets/overlaps.shp' OTHER='./assets/buildings_other.shp'
```

# As a library
It depends on the [geo](https://crates.io/crates/geo) crate. Import the following traits:
```rust
use topology_checker::rules::{MustNotHaveDangles, MustNotIntersect, MustNotOverlap};
```
These rules will now be implemented for vectors of certain geometries. For example, you could do:
```rust
use geo::{line_string, LineString};
use topology_checker::rules::MustNotHaveDangles;
let lines: Vec<LineString> = vec![line_string![(0., 0.), (1., 1.)], line_string![(1., 1.), (2., 2.)]];
let dangles: Vec<Point> = lines.must_not_have_dangles();

use topology_checker::rules::MustNotIntersect;
let intersections: (Vec<geo::Line>, Vec<geo::Point>) = lines.must_no_intesect();

use topology_checker::rules::MustNotOverlap;
use geo::polygon;
let polygons: Vec<Polygon> = vec![polygon![(0., 0.), (1., 0.), (1., 1.), (0., 1.), (0., 0.)], polygon![(1., 1.), (2., 1.), (2., 2.), (1., 2.), (1., 1.)]];
let overlaps: Vec<Polygon> = polygons.must_not_overlap();
let others: Vec<Polygon> = vec![polygon![(0., 0.), (1., 0.), (1., 1.), (0., 1.), (0., 0.)], polygon![(1., 1.), (2., 1.), (2., 2.), (1., 2.), (1., 1.)]];
let overlaps: Vec<Polygon> = polygons.must_not_overlap(others);
```