use crate::{
    util::{explode_linestrings, intersections},
    GeometryType,
};
use geo::{
    sweep::SweepPoint, BooleanOps, Contains, GeoFloat, HasDimensions, Intersects, Line, LineString,
    MultiPolygon, Point, Polygon,
};
use itertools::Itertools;
use rayon::iter::{ParallelBridge, ParallelIterator};
use rstar::RTree;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

pub trait MustNotOverlap<T: GeoFloat, I: GeometryType<T>, O: GeometryType<T>> {
    fn must_not_overlap(&self) -> Vec<O>;
    fn must_not_overlap_with(&self, other: Vec<I>) -> Vec<O>;
}

impl<T: GeoFloat + Send + Sync> MustNotOverlap<T, Polygon<T>, Polygon<T>> for Vec<Polygon<T>> {
    fn must_not_overlap(&self) -> Vec<Polygon<T>> {
        let lines: Vec<Line<T>> = self
            .iter()
            .par_bridge()
            .flat_map_iter(|polygon| polygon.exterior().lines())
            .collect();
        let (_, (proper, improper)) = intersections::<T, SweepPoint<T>, SweepPoint<T>>(lines);

        let mut points: BTreeSet<SweepPoint<T>> = improper
            .into_iter()
            .par_bridge()
            .filter(|point| {
                let buffer = Arc::new(Mutex::new(0));
                let counter = Arc::clone(&buffer);
                self.iter().par_bridge().for_each(move |polygon| {
                    if point.intersects(polygon) {
                        let mut counter = counter.lock().unwrap();
                        *counter += 1;
                    }
                });
                let buffer = *buffer.lock().unwrap();
                buffer > 1
            })
            .collect();
        points.extend(proper);

        let mut overlaps: Vec<Polygon<T>> = Vec::new();
        let mut combinations: Vec<(&Polygon<T>, &Polygon<T>)> = Vec::new();

        for intersection in points.into_iter() {
            let point: Point<T> = Point::new(intersection.x, intersection.y);
            let intersecting_polygons: Vec<&Polygon<T>> = self
                .iter()
                .par_bridge()
                .filter(|polygon| polygon.intersects(&point))
                .collect();
            for combination in intersecting_polygons
                .into_iter()
                .tuple_combinations::<(_, _)>()
            {
                if !combinations.contains(&combination)
                    && !combinations.contains(&(combination.1, combination.0))
                {
                    combinations.push(combination);
                }
            }
        }
        for combination in combinations.into_iter() {
            let overlap = combination.0.intersection(combination.1);
            if !overlap.is_empty() {
                overlaps.extend(overlap.0);
            }
        }
        overlaps
    }

    fn must_not_overlap_with(&self, other: Vec<Polygon<T>>) -> Vec<Polygon<T>> {
        let other = MultiPolygon::from_iter(other);
        self.into_iter()
            .par_bridge()
            .filter_map(|polygon| {
                let intersection =
                    other.intersection(&MultiPolygon::from_iter(std::iter::once(polygon.clone())));
                if !intersection.is_empty() {
                    return Some(intersection.0);
                }
                None
            })
            .flatten()
            .collect()
    }
}

impl<T: Send + Sync + GeoFloat> MustNotOverlap<T, LineString<T>, Line<T>> for Vec<LineString<T>> {
    fn must_not_overlap(&self) -> Vec<Line<T>> {
        let lines = explode_linestrings(self);
        let (line_intersections, ..) = intersections::<T, SweepPoint<T>, SweepPoint<T>>(lines);
        line_intersections
    }
    fn must_not_overlap_with(&self, others: Vec<LineString<T>>) -> Vec<Line<T>> {
        let lines: Vec<Line<T>> = explode_linestrings(self).into_iter().collect();
        let others: Vec<Line<T>> = explode_linestrings(&others).into_iter().collect();
        let lines_tree: RTree<Line<T>> = RTree::bulk_load(lines);
        let others_tree = RTree::bulk_load(others);
        let candidates = lines_tree.intersection_candidates_with_other_tree(&others_tree);
        candidates
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
