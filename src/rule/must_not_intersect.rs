use std::collections::BTreeSet;

use crate::{
    algorithm::merge_linestrings,
    util::{explode_linestrings, intersections, linestring_inner_points, sweep_points_to_points},
};
use geo::{sweep::SweepPoint, LineString, Point};
use itertools::Itertools;

pub trait MustNotIntersect<L, R> {
    fn must_not_intersect(&self) -> (Vec<L>, Vec<R>);
}

impl MustNotIntersect<LineString, Point> for Vec<LineString> {
    fn must_not_intersect(&self) -> (Vec<LineString>, Vec<Point>) {
        let mut endpoints = linestring_inner_points(self);
        endpoints.sort();
        let lines = explode_linestrings(self);
        let subset = endpoints
            .into_iter()
            .dedup_with_count()
            .filter_map(|(size, item)| {
                if size > 1 {
                    // Size > 1 means that the inner point (a Line endpoint) corresponds to 2 or more lines.
                    // If Size > 1, we care about the point. This is how we only select the intersections
                    // which are also LineString endpoints, which is what we want.
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
        // We extend the SweepPoint vector with the proper intersections (the intersections)
        // which are not Line or LineString endpoints.
        points.extend(proper);
        let points: Vec<Point> = sweep_points_to_points(points);
        (
            merge_linestrings(lines.into_iter().map_into().collect()),
            points,
        )
    }
}
