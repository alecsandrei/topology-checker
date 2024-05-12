use crate::{
    util::{explode_linestrings, intersections},
    GeometryType,
};
use geo::{
    sweep::SweepPoint, BooleanOps, Contains, GeoFloat, HasDimensions, Intersects, Line, LineString,
    LinesIter, Point, Polygon,
};
use rstar::RTree;

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
        polygons
            .intersection_candidates_with_other_tree(&polygons)
            .filter_map(|(polygon, other)| {
                if !std::ptr::addr_eq(polygon, other) && polygon.intersects(other) {
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
                    return Some(*line);
                }
                None
            })
            .collect()
    }
}

impl<T: Send + Sync + GeoFloat> MustNotOverlap<T, Point<T>, Point<T>> for Vec<Point<T>> {
    fn must_not_overlap(self) -> Vec<Point<T>> {
        let points = RTree::bulk_load(self);
        points
            .intersection_candidates_with_other_tree(&points)
            .filter_map(|(point, other)| {
                if !std::ptr::addr_eq(point, other) && point.intersects(other) {
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
                if point.eq(other) {
                    return Some(*point);
                }
                None
            })
            .collect()
    }
}

impl<T: Send + Sync + GeoFloat> MustNotSelfOverlap<T, LineString<T>, Line<T>>
    for Vec<LineString<T>>
{
    fn must_not_self_overlap(self) -> Vec<Line<T>> {
        self.into_iter()
            .flat_map(|linestring| {
                intersections::<T, SweepPoint<T>, SweepPoint<T>>(linestring.lines_iter()).0
            })
            .collect()
    }
}
