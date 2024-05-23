mod geometry;
mod io;

pub use geometry::{
    coords_to_points, explode_linestrings, flatten_linestrings, flatten_points, flatten_polygons,
    intersections, is_line, is_point, is_polygon, linestring_endpoints, linestring_inner_points,
    sweep_points_to_points,
};
pub use io::{open_dataset, create_dataset, geometries_to_file, GdalDrivers};
