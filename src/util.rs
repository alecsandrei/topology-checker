mod geometry;
mod io;

pub use geometry::{
    coords_to_points, explode_linestrings, flatten_linestrings, flatten_polygons, intersections,
    is_line, is_point, is_polygon, linestring_endpoints, linestring_inner_points,
    sweep_points_to_points, flatten_points
};
pub use io::{geometries_to_file, open_dataset, GdalDrivers};
