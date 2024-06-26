use std::collections::BTreeSet;

use crate::{
    util::{explode_linestrings, intersections, linestring_inner_points, sweep_points_to_points},
    TopologyError, TopologyResult,
};
use geo::{sweep::SweepPoint, GeoFloat, LineString, Point};
use itertools::Itertools;

pub trait MustNotIntersect<T: GeoFloat> {
    fn must_not_intersect(&self) -> TopologyResult<T>;
}

impl<T: GeoFloat + Send + Sync> MustNotIntersect<T> for Vec<LineString<T>> {
    fn must_not_intersect(&self) -> TopologyResult<T> {
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
        let (lines, (proper, improper)) = intersections::<T, SweepPoint<T>, SweepPoint<T>>(lines);
        let mut points: BTreeSet<SweepPoint<T>> = improper
            .into_iter()
            .filter(|point| subset.binary_search(&point).is_ok())
            .collect();
        // Extend with the proper intersections.
        points.extend(proper);
        let points: Vec<Point<T>> = sweep_points_to_points(points).into_iter().collect();
        let linestrings: Vec<LineString<T>> = lines.into_iter().map_into().collect();

        let mut geometry_errors = Vec::new();
        if !points.is_empty() {
            geometry_errors.push(TopologyError::Point(points))
        }
        if !linestrings.is_empty() {
            geometry_errors.push(TopologyError::LineString(linestrings))
        }
        if geometry_errors.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(geometry_errors)
        }
    }
}
