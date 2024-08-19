use crate::{TopologyError, TopologyResult};
use geo::{Contains, GeoFloat, LineString, Point, Polygon};
use rayon::iter::ParallelBridge;
use rayon::iter::ParallelIterator;
use rstar::RTree;

pub trait MustBeInside<T: GeoFloat + Send + Sync> {
    fn must_be_inside(self, other: Vec<Polygon<T>>) -> TopologyResult<T>;
}

// TODO for both point and linestring implementations:
// try to eliminate the clone in Some(*point) and Some(linestring.clone())

impl<T: GeoFloat + Send + Sync> MustBeInside<T> for Vec<Point<T>> {
    fn must_be_inside(self, other: Vec<Polygon<T>>) -> TopologyResult<T> {
        let points = RTree::bulk_load(self);
        let polygons: RTree<Polygon<T>> = RTree::bulk_load(other.into_iter().collect());
        let inside_points: Vec<Point<T>> = points
            .intersection_candidates_with_other_tree(&polygons)
            .par_bridge()
            .filter_map(|(point, polygon)| {
                if polygon.contains(point) {
                    Some(*point)
                } else {
                    None
                }
            })
            .collect();
        let outside_points: Vec<Point<T>> = points
            .into_iter()
            .par_bridge()
            .filter(|point| !inside_points.contains(&point))
            .collect();
        if outside_points.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::Point(outside_points)])
        }
    }
}

impl<T: GeoFloat + Send + Sync> MustBeInside<T> for Vec<LineString<T>> {
    fn must_be_inside(self, other: Vec<Polygon<T>>) -> TopologyResult<T> {
        let linestrings = RTree::bulk_load(self);
        let polygons = RTree::bulk_load(other.into_iter().collect());
        let inside_linestrings: Vec<LineString<T>> = linestrings
            .intersection_candidates_with_other_tree(&polygons)
            .par_bridge()
            .filter_map(|(linestring, polygon)| {
                if polygon.contains(linestring) {
                    Some(linestring.clone())
                } else {
                    None
                }
            })
            .collect();
        let outside_linestrings: Vec<LineString<T>> = linestrings
            .into_iter()
            .par_bridge()
            .filter(|line| !inside_linestrings.contains(&line))
            .collect();
        if outside_linestrings.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::LineString(outside_linestrings)])
        }
    }
}

#[cfg(test)]
mod tests {

    use geo::polygon;

    use super::*;

    mod points {
        use geo::{point, Centroid};

        use super::*;

        #[test]
        fn valid() {
            let polygons = vec![
                polygon![(x: 0., y: 0.), (x: 1., y: 0.), (x: 1., y: 1.), (x: 0., y: 1.), (x: 0., y: 0.)],
                polygon![(x: 0.25, y: 0.25), (x: 0.75, y: 0.25), (x: 0.75, y: 0.75), (x: 0.25, y: 0.75), (x: 0.25, y: 0.25)],
            ];
            let input = vec![
                point! {x: 0.01, y: 0.01},       // falls close to the edge
                point! {x: 0.5, y: 0.5},         // does not fall on the edge
                polygons[0].centroid().unwrap(), // the polygon centroid
                polygons[1].centroid().unwrap(), // the polygon centroid
            ];
            let result = input.must_be_inside(polygons);
            assert!(result.is_valid());
        }

        #[test]
        fn invalid() {
            let polygons = vec![
                polygon![(x: 0., y: 0.), (x: 1., y: 0.), (x: 1., y: 1.), (x: 0., y: 1.), (x: 0., y: 0.)],
                polygon![(x: 0.25, y: 0.25), (x: 0.75, y: 0.25), (x: 0.75, y: 0.75), (x: 0.25, y: 0.75), (x: 0.25, y: 0.25)],
            ];
            let input = vec![
                point! {x: 0., y: 0.},   // falls on the edge
                point! {x: -1., y: -1.}, // falls outside
                point! {x: 0.5, y: 0.5}, // is inside
                point! {x: 999., y: 999.},
            ];
            let invalid_points = vec![input[3], input[1], input[0]];
            let result = input.must_be_inside(polygons);
            let point_errors: &TopologyError<f64> = result.unwrap_err_point();
            if let TopologyError::Point(point_errors) = point_errors {
                for point in point_errors {
                    assert!(invalid_points.contains(point))
                }
            }
        }
    }

    mod linestrings {

        #[test]
        fn valid() {
            // TODO
        }

        #[test]
        fn invalid() {
            // TOOD
        }
    }
}
