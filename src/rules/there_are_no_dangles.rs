use crate::utils::{
    flatten_lines, flatten_linestrings, intersections, linestring_endpoints, sweep_points_to_points,
};
use geo::{sweep::SweepPoint, LineString, Point};
use itertools::Itertools;

pub fn there_are_no_dangles(linestrings: Vec<&LineString>) -> Vec<Point> {
    // We find dangles by elimination from the LineString endpoints
    // BinaryHeap those points that are intersections.
    let endpoints = linestring_endpoints(&linestrings);
    let (_, (_, improper)) =
        intersections::<f64, SweepPoint<f64>, SweepPoint<f64>>(flatten_lines(linestrings));
    let endpoints = endpoints
        .into_iter()
        .filter_map(|point| {
            if !improper.contains(&point) {
                return Some(point);
            }
            None
        })
        .collect_vec();
    sweep_points_to_points(endpoints)
}
