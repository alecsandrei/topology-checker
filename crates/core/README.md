# topology-checker-core

A GDAL-free, planar topology validator for `geo::Geometry<f64>` datasets.

```rust
use topology_checker_core::{geo::{line_string, Geometry}, Layer, Rule, Topology};

let mut topology = Topology::new(0.001)?;
let roads = topology.add_layer(Layer::from_geometries("roads", [
    Geometry::from(line_string![(x: 0., y: 0.), (x: 1., y: 0.)]),
    Geometry::from(line_string![(x: 1., y: 0.), (x: 2., y: 0.)]),
]));
// The line rules are available after constructing the shared line graph.
assert_eq!(topology.layer(roads)?.name(), "roads");
# Ok::<(), topology_checker_core::Error>(())
```
