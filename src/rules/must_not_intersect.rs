use std::collections::BinaryHeap;

use crate::utils::{
    flatten_lines, flatten_linestrings, intersections, linestring_inner_points,
    sweep_points_to_points,
};
use geo::{sweep::SweepPoint, LineString, Line, Point};
use itertools::Itertools;
use rayon::iter::{ParallelBridge, ParallelIterator};

pub fn must_not_intersect(linestrings: Vec<&LineString>) -> (Vec<Line>, Vec<Point>) {
    let endpoints = linestring_inner_points(&linestrings);
    let lines = flatten_lines(linestrings);
    let subset = endpoints
        .into_iter()
        .dedup_with_count()
        .filter_map(|(size, item)| {
            if size > 1 {
                // Size > 1 means that the endpoint corresponds to 2 or more lines.
                // If Size > 1, we care about the point. This is how we only select the intersections
                // which are also LineString endpoints, which is what we want.
                return Some(item);
            }
            None
        })
        .collect_vec();
    let (lines, (proper, improper)) = intersections::<f64, SweepPoint<f64>, SweepPoint<f64>>(lines);
    let mut points: BinaryHeap<SweepPoint<f64>> = improper
        .into_iter()
        .filter(|point| subset.binary_search(&point).is_ok())
        .collect();
    // We extend the SweepPoint heap with the proper intersections (the intersections)
    // which are not Line or LineString endpoints.
    points.extend(proper);
    let points: Vec<Point> = sweep_points_to_points(points.into());
    (
        lines,
        points
            .into_iter()
            .par_bridge()
            .map(|sweep_point| sweep_point.into())
            .collect(),
    )
}
