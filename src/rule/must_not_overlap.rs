use crate::{
    util::{explode_linestrings, intersections},
    TopologyError, GeometryType, TopologyResult,
};
use geo::{
    sweep::SweepPoint, BooleanOps, Contains, GeoFloat, HasDimensions,
    Intersects, Line, LineString, LinesIter, Point, Polygon,
};
use itertools::Itertools;
use rstar::RTree;
use std::ptr::addr_of;

pub trait MustNotOverlap<T: GeoFloat, I: GeometryType<T>, O: GeometryType<T>> {
    fn must_not_overlap(self) -> TopologyResult<T>;
    fn must_not_overlap_with(self, other: Vec<I>) -> TopologyResult<T>;
}

pub trait MustNotSelfOverlap<T: GeoFloat> {
    fn must_not_self_overlap(self) -> TopologyResult<T>;
}

impl<T: GeoFloat + Send + Sync> MustNotOverlap<T, Polygon<T>, Polygon<T>> for Vec<Polygon<T>> {
    fn must_not_overlap(self) -> TopologyResult<T> {
        let polygons = RTree::bulk_load(self);
        // We make this addresses container to avoid duplicate geometries.
        // The 'intersection_candidates_with_other_tree' method will yield both
        // (Polygon1, Polygon2) and (Polygon2, Polygon1).
        // By comparing addresses we make a lightweight assurance that we have not already
        // visited (Polygon1, Polygon2).
        // TODO: implement this addresses container for other 'must_not_overlap'.
        let mut addresses = Vec::new();
        let geometry_errors: Vec<_> = polygons
            .intersection_candidates_with_other_tree(&polygons)
            .filter_map(|(polygon, other)| {
                let address = (addr_of!(*polygon), addr_of!(*other));
                if !std::ptr::addr_eq(polygon, other)
                    && !addresses.contains(&(address.1, address.0))
                    && polygon.intersects(other)
                {
                    addresses.push(address);
                    let intersection = polygon.intersection(other);
                    if !intersection.is_empty() {
                        return Some(intersection.into_iter());
                    }
                }
                None
            })
            .flatten()
            .collect();
        if geometry_errors.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::Polygon(geometry_errors)])
        }
    }

    fn must_not_overlap_with(self, others: Vec<Polygon<T>>) -> TopologyResult<T> {
        let polygons = RTree::bulk_load(self);
        let others = RTree::bulk_load(others);
        let geometry_errors: Vec<_> = polygons
            .intersection_candidates_with_other_tree(&others)
            .filter_map(|(polygon, other)| {
                if polygon.intersects(other) {
                    let intersection = polygon.intersection(other);
                    if !intersection.is_empty() {
                        return Some(intersection.into_iter());
                    }
                }
                None
            })
            .flatten()
            .collect();
        if geometry_errors.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::Polygon(geometry_errors)])
        }
    }
}

impl<T: Send + Sync + GeoFloat> MustNotOverlap<T, LineString<T>, Line<T>> for Vec<LineString<T>> {
    fn must_not_overlap(self) -> TopologyResult<T> {
        let lines = explode_linestrings(&self);
        let (geometry_errors, ..) = intersections::<T, SweepPoint<T>, SweepPoint<T>>(lines);
        if geometry_errors.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::LineString(
                geometry_errors.into_iter().map_into().collect(),
            )])
        }
    }

    fn must_not_overlap_with(self, others: Vec<LineString<T>>) -> TopologyResult<T> {
        let lines: Vec<Line<T>> = explode_linestrings(&self).into_iter().collect();
        let others: Vec<Line<T>> = explode_linestrings(&others).into_iter().collect();
        let lines_tree: RTree<Line<T>> = RTree::bulk_load(lines);
        let others_tree = RTree::bulk_load(others);
        let geometry_errors: Vec<_> = lines_tree
            .intersection_candidates_with_other_tree(&others_tree)
            .into_iter()
            .filter_map(|(line, other)| {
                if line.contains(other) {
                    return Some(*other);
                }
                None
            })
            .collect();
        if geometry_errors.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::LineString(
                geometry_errors.into_iter().map_into().collect(),
            )])
        }
    }
}

impl<T: Send + Sync + GeoFloat> MustNotOverlap<T, Point<T>, Point<T>> for Vec<Point<T>> {
    fn must_not_overlap(self) -> TopologyResult<T> {
        let points = RTree::bulk_load(self);
        // We make this addresses container to avoid duplicate geometries.
        // The 'intersection_candidates_with_other_tree' method will yield both
        // (Polygon1, Polygon2) and (Polygon2, Polygon1).
        // By comparing addresses we make a lightweight assurance that we have not already
        // visited (Polygon1, Polygon2).
        // TODO: implement this addresses container for other 'must_not_overlap'.
        let mut addresses = Vec::new();
        let geometry_errors: Vec<_> = points
            .intersection_candidates_with_other_tree(&points)
            .filter_map(|(point, other)| {
                let address = (addr_of!(*point), addr_of!(*other));
                if !std::ptr::addr_eq(point, other)
                    && !addresses.contains(&(address.1, address.0))
                    && point.intersects(other)
                {
                    addresses.push(address);
                    return Some(*point);
                }
                None
            })
            .collect();
        if geometry_errors.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::Point(
                geometry_errors.into_iter().map_into().collect(),
            )])
        }
    }

    fn must_not_overlap_with(self, others: Vec<Point<T>>) -> TopologyResult<T> {
        let points = RTree::bulk_load(self);
        let others = RTree::bulk_load(others);
        let geometry_errors: Vec<_> = points
            .intersection_candidates_with_other_tree(&others)
            .into_iter()
            .filter_map(|(point, other)| {
                if point.intersects(other) {
                    return Some(*point);
                }
                None
            })
            .collect();
        if geometry_errors.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::Point(
                geometry_errors.into_iter().map_into().collect(),
            )])
        }
    }
}

impl<T: GeoFloat> MustNotSelfOverlap<T> for Vec<LineString<T>> {
    fn must_not_self_overlap(self) -> TopologyResult<T> {
        let geometry_errors: Vec<_> = self
            .into_iter()
            .flat_map(|linestring| {
                let lines = RTree::bulk_load(linestring.lines_iter().collect());
                lines
                    .intersection_candidates_with_other_tree(&lines)
                    .filter_map(|(line, other)| {
                        if !std::ptr::addr_eq(line, other) && line.contains(other) {
                            return Some(*other);
                        }
                        None
                    })
                    .collect_vec()
            })
            .collect();
        if geometry_errors.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::LineString(
                geometry_errors.into_iter().map_into().collect(),
            )])
        }
    }
}

#[cfg(test)]
mod tests {

    use geo::line_string;
    use geo::polygon;

    use super::*;

    mod points {
        use super::*;
        use geo::point;

        #[test]
        fn overlap() {
            let input = vec![
                point! { x: 181.2, y: 51.79 },
                point! { x: 181.2, y: 51.79 },
                point! { x: 184.0, y: 53.0 },
            ];
            let output = vec![point! { x: 181.2, y: 51.79 }];
            assert_eq!(
                input.must_not_overlap().unwrap_err_point(),
                &TopologyError::Point(output)
            );
        }

        #[test]
        fn overlap_with() {
            let input1 = vec![point! { x: 181.2, y: 51.79 }, point! { x: 184.0, y: 53.0 }];
            let input2 = vec![point! { x: 181.2, y: 51.79 }];
            let output = vec![point! { x: 181.2, y: 51.79 }];
            assert_eq!(
                input1.must_not_overlap_with(input2).unwrap_err_point(),
                &TopologyError::Point(output)
            );
        }
    }

    mod line_strings {
        use super::*;
        #[test]
        fn self_overlap() {
            let input = vec![line_string![(x: 1., y: 1.), (x: 4., y: 4.), (x: 2., y: 2.)]];
            let output = vec![line_string![(x: 4., y: 4.), (x: 2., y: 2.)]];
            assert_eq!(
                input.must_not_self_overlap().unwrap_err_linestring(),
                &TopologyError::LineString(output)
            );
        }

        #[test]
        fn overlap() {
            let input = vec![
                line_string![(x: 1., y: 1.), (x: 4., y: 4.)],
                line_string![(x: 4., y: 4.), (x: 2., y: 2.)],
            ];
            let output = vec![line_string![(x: 2., y: 2.), (x: 4., y: 4.)]];
            assert_eq!(
                input.must_not_overlap().unwrap_err_linestring(),
                &TopologyError::LineString(output)
            );
        }

        #[test]
        fn overlap_with() {
            let input1 = vec![line_string![(x: 1., y: 1.), (x: 4., y: 4.)]];
            let input2 = vec![line_string![(x: 4., y: 4.), (x: 2., y: 2.)]];
            let output = vec![line_string![(x: 4., y: 4.), (x: 2., y: 2.)]];
            assert_eq!(
                input1.must_not_overlap_with(input2).unwrap_err_linestring(),
                &TopologyError::LineString(output)
            );
        }
    }

    mod polygons {
        use super::*;

        #[test]
        fn overlap() {
            let input = vec![
                polygon![(x: 0., y: 0.), (x: 1., y: 0.), (x: 1., y: 1.), (x: 0., y: 1.), (x: 0., y: 0.)],
                polygon![(x: 0.25, y: 0.25), (x: 0.75, y: 0.25), (x: 0.75, y: 0.75), (x: 0.25, y: 0.75), (x: 0.25, y: 0.25)],
            ];
            let output = vec![input[0].intersection(&input[1]).into_iter().next().unwrap()];
            assert_eq!(
                input.must_not_overlap().unwrap_err_polygon(),
                &TopologyError::Polygon(output)
            );
        }

        #[test]
        fn overlap_with() {
            let input1 = vec![
                polygon![(x: 0., y: 0.), (x: 1., y: 0.), (x: 1., y: 1.), (x: 0., y: 1.), (x: 0., y: 0.)],
            ];
            let input2 = vec![
                polygon![(x: 0.25, y: 0.25), (x: 0.75, y: 0.25), (x: 0.75, y: 0.75), (x: 0.25, y: 0.75), (x: 0.25, y: 0.25)],
            ];
            let output = vec![input1[0]
                .intersection(&input2[0])
                .into_iter()
                .next()
                .unwrap()];
            assert_eq!(
                input1.must_not_overlap_with(input2).unwrap_err_polygon(),
                &TopologyError::Polygon(output)
            );
        }
    }
}
