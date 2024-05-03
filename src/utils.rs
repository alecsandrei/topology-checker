mod geometry;

pub use geometry::{
    coords_to_points, flatten_lines, flatten_linestrings, intersections, linestring_endpoints,
    linestring_inner_points, sweep_points_to_points, flatten_polygons
};
