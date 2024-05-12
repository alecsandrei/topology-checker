use crate::{
    util::{explode_linestrings, intersections},
    GeometryType,
};
use geo::{
    sweep::SweepPoint, BooleanOps, Contains, GeoFloat, HasDimensions, Intersects, Line, LineString,
    LinesIter, Point, Polygon,
};
use itertools::Itertools;
use rstar::RTree;
use std::ptr::addr_of;

pub trait MustNotOverlap<T: GeoFloat, I: GeometryType<T>, O: GeometryType<T>> {
    fn must_not_overlap(self) -> Vec<O>;
    fn must_not_overlap_with(self, other: Vec<I>) -> Vec<O>;
}

pub trait MustNotSelfOverlap<T: GeoFloat, I: GeometryType<T>, O: GeometryType<T>> {
    fn must_not_self_overlap(self) -> Vec<O>;
}

impl<T: GeoFloat + Send + Sync> MustNotOverlap<T, Polygon<T>, Polygon<T>> for Vec<Polygon<T>> {
    fn must_not_overlap(self) -> Vec<Polygon<T>> {
        let polygons = RTree::bulk_load(self);
        // We make this addresses container to avoid duplicate geometries.
        // The 'intersection_candidates_with_other_tree' method will yield both
        // (Polygon1, Polygon2) and (Polygon2, Polygon1).
        // By comparing addresses we make a lightweight assurance that we have not already
        // visited (Polygon1, Polygon2).
        // TODO: implement this addresses container for other 'must_not_overlap'.
        let mut addresses = Vec::new();
        polygons
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
            .collect()
    }

    fn must_not_overlap_with(self, others: Vec<Polygon<T>>) -> Vec<Polygon<T>> {
        let polygons = RTree::bulk_load(self);
        let others = RTree::bulk_load(others);
        polygons
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
            .collect()
    }
}

impl<T: Send + Sync + GeoFloat> MustNotOverlap<T, LineString<T>, Line<T>> for Vec<LineString<T>> {
    fn must_not_overlap(self) -> Vec<Line<T>> {
        let lines = explode_linestrings(&self);
        let (line_intersections, ..) = intersections::<T, SweepPoint<T>, SweepPoint<T>>(lines);
        line_intersections
    }
    fn must_not_overlap_with(self, others: Vec<LineString<T>>) -> Vec<Line<T>> {
        let lines: Vec<Line<T>> = explode_linestrings(&self).into_iter().collect();
        let others: Vec<Line<T>> = explode_linestrings(&others).into_iter().collect();
        let lines_tree: RTree<Line<T>> = RTree::bulk_load(lines);
        let others_tree = RTree::bulk_load(others);
        lines_tree
            .intersection_candidates_with_other_tree(&others_tree)
            .into_iter()
            .filter_map(|(line, other)| {
                if line.contains(other) {
                    return Some(*other);
                }
                None
            })
            .collect()
    }
}

impl<T: Send + Sync + GeoFloat> MustNotOverlap<T, Point<T>, Point<T>> for Vec<Point<T>> {
    fn must_not_overlap(self) -> Vec<Point<T>> {
        let points = RTree::bulk_load(self);
        // We make this addresses container to avoid duplicate geometries.
        // The 'intersection_candidates_with_other_tree' method will yield both
        // (Polygon1, Polygon2) and (Polygon2, Polygon1).
        // By comparing addresses we make a lightweight assurance that we have not already
        // visited (Polygon1, Polygon2).
        // TODO: implement this addresses container for other 'must_not_overlap'.
        let mut addresses = Vec::new();
        points
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
            .collect()
    }

    fn must_not_overlap_with(self, others: Vec<Point<T>>) -> Vec<Point<T>> {
        let points = RTree::bulk_load(self);
        let others = RTree::bulk_load(others);
        points
            .intersection_candidates_with_other_tree(&others)
            .into_iter()
            .filter_map(|(point, other)| {
                if point.intersects(other) {
                    return Some(*point);
                }
                None
            })
            .collect()
    }
}

impl<T: GeoFloat> MustNotSelfOverlap<T, LineString<T>, Line<T>> for Vec<LineString<T>> {
    fn must_not_self_overlap(self) -> Vec<Line<T>> {
        self.into_iter()
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
            .collect()
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
            assert_eq!(input.must_not_overlap(), output);
        }

        #[test]
        fn overlap_with() {
            let input1 = vec![point! { x: 181.2, y: 51.79 }, point! { x: 184.0, y: 53.0 }];
            let input2 = vec![point! { x: 181.2, y: 51.79 }];
            let output = vec![point! { x: 181.2, y: 51.79 }];
            assert_eq!(input1.must_not_overlap_with(input2), output);
        }
    }

    #[cfg(test)]
    mod line_strings {
        use super::*;
        #[test]
        fn self_overlap() {
            let input = vec![line_string![(x: 1., y: 1.), (x: 4., y: 4.), (x: 2., y: 2.)]];
            let output = vec![Line::new((4., 4.), (2., 2.))];
            assert_eq!(input.must_not_self_overlap(), output);
        }

        #[test]
        fn overlap() {
            let input = vec![
                line_string![(x: 1., y: 1.), (x: 4., y: 4.)],
                line_string![(x: 4., y: 4.), (x: 2., y: 2.)],
            ];
            let output = vec![Line::new((2., 2.), (4., 4.))];
            assert_eq!(input.must_not_overlap(), output);
        }

        #[test]
        fn overlap_with() {
            let input1 = vec![line_string![(x: 1., y: 1.), (x: 4., y: 4.)]];
            let input2 = vec![line_string![(x: 4., y: 4.), (x: 2., y: 2.)]];
            let output = vec![Line::new((4., 4.), (2., 2.))];
            assert_eq!(input1.must_not_overlap_with(input2), output);
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
            let output = input[0].intersection(&input[1]).into_iter().next().unwrap();
            assert_eq!(*input.must_not_overlap().first().unwrap(), output);
        }

        #[test]
        fn overlap_with() {
            let input1 = vec![
                polygon![(x: 0., y: 0.), (x: 1., y: 0.), (x: 1., y: 1.), (x: 0., y: 1.), (x: 0., y: 0.)],
            ];
            let input2 = vec![
                polygon![(x: 0.25, y: 0.25), (x: 0.75, y: 0.25), (x: 0.75, y: 0.75), (x: 0.25, y: 0.75), (x: 0.25, y: 0.25)],
            ];
            let output = input1[0]
                .intersection(&input2[0])
                .into_iter()
                .next()
                .unwrap();
            assert_eq!(
                *input1.must_not_overlap_with(input2).first().unwrap(),
                output
            );
        }
    }
}
