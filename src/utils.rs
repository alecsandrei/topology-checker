mod geometry;
mod io;

pub use geometry::{
    coords_to_points, flatten_lines, flatten_linestrings, flatten_polygons, intersections,
    linestring_endpoints, linestring_inner_points, sweep_points_to_points,
};
pub use io::{geometries_to_file, open_dataset, GdalDrivers};
