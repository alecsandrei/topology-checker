use crate::utils::{flatten_lines, intersections, linestring_endpoints, sweep_points_to_points};
use geo::{sweep::SweepPoint, LineString, Point};
use itertools::Itertools;

pub trait MustNotHaveDangles {
    fn must_not_have_dangles(&self) -> Vec<Point>;
}

impl MustNotHaveDangles for Vec<LineString> {
    fn must_not_have_dangles(&self) -> Vec<Point> {
        // We find dangles by elimination from the LineString endpoints
        // BinaryHeap those points that are intersections.
        let endpoints = linestring_endpoints(self);
        let (_, (_, improper)) =
            intersections::<f64, SweepPoint<f64>, SweepPoint<f64>>(flatten_lines(self));
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
}
