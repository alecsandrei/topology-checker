use crate::utils::{flatten_lines, flatten_linestring, linestring_endpoints, intersections, sweep_points_to_points};
use geo::{Point, Geometry, Line, sweep::SweepPoint};
use itertools::Itertools;

pub fn there_are_no_dangles_improved(lines: Vec<Geometry>) -> Vec<Point> {
    let linestrings = flatten_linestring(lines);
    let endpoints = linestring_endpoints(&linestrings);
    let (_, (proper, improper)) = intersections(flatten_lines(linestrings));

    let improper: Vec<SweepPoint<f64>> = improper.into_iter().map(|point| point.into()).collect();

    let endpoints = endpoints
        .into_iter()
        .filter_map(|point| {
            if improper.binary_search(&point).is_err() {
                return Some(point);
            }
            None
        })
        .collect_vec();

    println!("{}", endpoints.len());

    sweep_points_to_points(endpoints)

}
