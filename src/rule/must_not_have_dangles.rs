use crate::{
    util::{explode_linestrings, intersections, linestring_endpoints, sweep_points_to_points},
    TopologyError, TopologyResult,
};
use geo::{sweep::SweepPoint, GeoFloat, LineString};
use itertools::Itertools;

pub trait MustNotHaveDangles<T: GeoFloat> {
    fn must_not_have_dangles(&self) -> TopologyResult<T>;
}

impl<T: GeoFloat + Send + Sync> MustNotHaveDangles<T> for Vec<LineString<T>> {
    fn must_not_have_dangles(&self) -> TopologyResult<T> {
        // We find dangles by elimination from the LineString endpoints
        // the points that are intersections.
        let endpoints = linestring_endpoints(self);
        let (_, (_, improper)) =
            intersections::<T, SweepPoint<T>, SweepPoint<T>>(explode_linestrings(self));
        let endpoints = endpoints
            .into_iter()
            .filter_map(|point| {
                if !improper.contains(&point) {
                    return Some(point);
                }
                None
            })
            .collect_vec();
        let geometry_errors: Vec<_> = sweep_points_to_points(endpoints).into_iter().collect();
        // REcheck logic
        if geometry_errors.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::Point(geometry_errors)])
        }
    }
}


// TODO: add tests