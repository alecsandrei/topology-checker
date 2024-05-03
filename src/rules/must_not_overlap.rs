use crate::rules::must_not_intersect;
use crate::utils::{flatten_polygons, intersections, sweep_points_to_points};
use geo::sweep::SweepPoint;
use geo::{Area, BooleanOps};
use geo::{Geometry, Intersects, Line, MultiPolygon, Point, Polygon};
use itertools::Itertools;
use rayon::iter::{ParallelBridge, ParallelIterator};

pub fn must_not_overlap(geometries: Vec<Geometry>) -> Vec<MultiPolygon> {
    let polygons = flatten_polygons(geometries);
    let lines: Vec<Line> = polygons
        .iter()
        .par_bridge()
        .map(|polygon| polygon.exterior().lines())
        .flatten_iter()
        .collect();
    // let (collinear, intersections) = must_not_intersect(lines);
    let (_, (proper, improper)) = intersections::<f64, SweepPoint<f64>, SweepPoint<f64>>(lines);

    let mut overlaps: Vec<MultiPolygon> = Vec::new();

    for intersection in improper.into_iter().chain(proper.into_iter()) {
        let point: Point = Point::new(intersection.x, intersection.y);
        let intersecting_polygons: Vec<&Polygon> = polygons
            .iter()
            .par_bridge()
            .filter(|polygon| polygon.intersects(&point))
            .collect();
        if intersecting_polygons.len() < 2 {
            continue;
        }
        for combination in intersecting_polygons
            .into_iter()
            .tuple_combinations::<(_, _)>()
        {
            let overlap = combination.0.intersection(combination.1);
            if overlap.signed_area() != 0.0 {
                overlaps.push(overlap);
            }
        }
    }
    overlaps
}
