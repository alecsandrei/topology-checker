use std::collections::BTreeSet;

use crate::util::{
    explode_linestrings, intersections, linestring_inner_points, sweep_points_to_points,
};
use geo::{sweep::SweepPoint, Line, LineString, Point};
use itertools::Itertools;

pub trait MustNotIntersect<L, R> {
    fn must_not_intersect(&self) -> (Vec<L>, Vec<R>);
}

impl MustNotIntersect<Line, Point> for Vec<LineString> {
    fn must_not_intersect(&self) -> (Vec<Line>, Vec<Point>) {
        let mut endpoints = linestring_inner_points(self);
        endpoints.sort();
        let lines = explode_linestrings(self);
        let subset = endpoints
            .into_iter()
            .dedup_with_count()
            .filter_map(|(size, item)| {
                // The inner point (a Line endpoint) corresponds to 2 or more lines.
                // If Size > 1, we care about the point. This is how we only select the intersections
                // which are also LineString endpoints, which is what we want.
                if size > 1 {
                    return Some(item);
                }
                None
            })
            .collect_vec();
        let (lines, (proper, improper)) =
            intersections::<f64, SweepPoint<f64>, SweepPoint<f64>>(lines);
        let mut points: BTreeSet<SweepPoint<f64>> = improper
            .into_iter()
            .filter(|point| subset.binary_search(&point).is_ok())
            .collect();
        // Extend with the proper intersections.
        points.extend(proper);
        let points: Vec<Point> = sweep_points_to_points(points);
        (lines.into_iter().map_into().collect(), points)
    }
}
