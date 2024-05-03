use crate::utils::{
    coords_to_points, flatten_lines, flatten_linestring, intersections, linestring_inner_points, sweep_points_to_points
};
use geo::{sweep::SweepPoint, Coord, Geometry, Line, Point};
use itertools::Itertools;
use rayon::iter::{ParallelBridge, ParallelIterator};

pub fn must_not_intersect(lines: Vec<Geometry>) -> (Vec<Line>, Vec<Point>) {
    let linestrings = flatten_linestring(lines);
    let endpoints = linestring_inner_points(&linestrings);
    let lines = flatten_lines(linestrings);
    let subset = endpoints
        .into_iter()
        .dedup_with_count()
        .filter_map(|(size, item)| {
            if size >= 2 {
                return Some(item);
            }
            None
        })
        .collect_vec();
    let (lines, (proper, improper)) = intersections(lines);
    let mut points: Vec<Coord> = improper
        .into_iter()
        .filter(|point| {
            let point: SweepPoint<f64> = <Coord as Into<SweepPoint<f64>>>::into(*point);
            subset.binary_search(&point).is_ok()
        })
        .collect();

    points.extend(proper);

    println!("{}, {}", lines.len(), points.len());
    let points: Vec<Point> = coords_to_points(points);
    (
        lines,
        points
            .into_iter()
            .par_bridge()
            .map(|sweeppoint| sweeppoint.into())
            .collect(),
    )
}
