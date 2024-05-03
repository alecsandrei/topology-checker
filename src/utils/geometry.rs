use geo::{
    algorithm::LineIntersection,
    sweep::{Intersections, SweepPoint},
    Coord, GeoFloat, Geometry, Line, LineString, LinesIter, Point,
    Polygon
};
use itertools::{Either, Itertools};
use rayon::{iter::ParallelIterator, prelude::*};
use std::collections::{BTreeSet, BinaryHeap};

pub fn flatten_linestrings(geometries: Vec<Geometry>) -> Vec<LineString> {
    geometries
        .into_iter()
        .par_bridge()
        .flat_map_iter(|geometry| match geometry {
            geo_types::Geometry::LineString(linestring) => {
                Box::new(std::iter::once(linestring)) as Box<dyn Iterator<Item = LineString>>
            }
            geo_types::Geometry::MultiLineString(multilinestring) => {
                Box::new(multilinestring.into_iter()) as Box<dyn Iterator<Item = LineString>>
            }
            _ => panic!("Unallowed geometries found."),
        })
        .collect()
}

pub fn flatten_polygons(geometries: Vec<Geometry>) -> Vec<Polygon> {
    geometries
        .into_iter()
        .par_bridge()
        .flat_map_iter(|geometry| match geometry {
            geo_types::Geometry::Polygon(linestring) => {
                Box::new(std::iter::once(linestring)) as Box<dyn Iterator<Item = Polygon>>
            }
            geo_types::Geometry::MultiPolygon(multilinestring) => {
                Box::new(multilinestring.into_iter()) as Box<dyn Iterator<Item = Polygon>>
            }
            _ => panic!("Unallowed geometries found."),
        })
        .collect()
}

pub fn flatten_lines(linestrings: Vec<&LineString>) -> Vec<Line> {
    linestrings
        .iter()
        .par_bridge()
        .flat_map_iter(|linestring| linestring.lines_iter())
        .collect()
}

pub fn linestring_inner_points<T>(linestring: &Vec<&LineString<T>>) -> BinaryHeap<SweepPoint<T>>
where
    T: GeoFloat,
{
    // Provides an ordered vector of inner points (points that are not endpoints).
    let mut heap: BinaryHeap<SweepPoint<T>> = BinaryHeap::new();
    for line in linestring.into_iter() {
        for coord in &line.0[1..line.0.len() - 1] {
            let point: SweepPoint<T> = <Coord<T> as Into<SweepPoint<T>>>::into(*coord);
            heap.push(point);
        }
    }
    heap
}

pub fn linestring_endpoints<T>(linestring: &Vec<&LineString<T>>) -> BinaryHeap<SweepPoint<T>>
where
    T: GeoFloat,
    Coord<T>: From<Point<T>>,
{
    // Provides an ordered vector of endpoints (points that are not inner points).
    let mut heap: BinaryHeap<SweepPoint<T>> = BinaryHeap::new();
    for line in linestring.into_iter() {
        let mut points = line.points();
        heap.push(<Point<T> as Into<SweepPoint<T>>>::into(
            points.next().unwrap(),
        ));
        heap.push(<Point<T> as Into<SweepPoint<T>>>::into(
            points.next_back().unwrap(),
        ));
    }
    heap
}

pub fn intersections<T, L, R>(
    lines: Vec<Line<T>>,
) -> (
    Vec<Line<T>>,
    (BTreeSet<SweepPoint<T>>, BTreeSet<SweepPoint<T>>),
)
where
    T: GeoFloat,
    L: From<Coord<T>>,
    R: From<Coord<T>>,
    BTreeSet<SweepPoint<T>>: Extend<L>,
    BTreeSet<SweepPoint<T>>: Extend<R>,
{
    // The intersections of lines.
    // Returns a tuple of collinear lines, unique proper single points and unique improper single points.
    let intersections = Intersections::from_iter(lines).collect::<Vec<_>>();
    let (lines, points): (Vec<_>, Vec<_>) = intersections
        .into_iter()
        .map(|vector| vector.2)
        .partition(|intersection| match intersection {
            LineIntersection::Collinear { .. } => true,
            LineIntersection::SinglePoint { .. } => false,
        });

    let lines: Vec<Line<T>> = lines
        .into_iter()
        .map(|line_intersection| {
            if let LineIntersection::Collinear { intersection } = line_intersection {
                intersection
            } else {
                unreachable!()
            }
        })
        .collect();

    let points: (BTreeSet<SweepPoint<T>>, BTreeSet<SweepPoint<T>>) = points
        .into_iter()
        .partition_map(|points_intersection| match points_intersection {
            LineIntersection::SinglePoint {
                intersection,
                is_proper: true,
            } => Either::Left::<L, R>(intersection.into()),
            LineIntersection::SinglePoint {
                intersection,
                is_proper: false,
            } => Either::Right::<L, R>(intersection.into()),
            _ => unreachable!(),
        });
    (lines, points)
}

pub fn coords_to_points<T>(coords: Vec<Coord>) -> Vec<Point> {
    coords
        .into_iter()
        .par_bridge()
        .map(|coord| coord.into())
        .collect()
}

pub fn sweep_points_to_points<T>(sweep_points: Vec<SweepPoint<T>>) -> Vec<Point<T>>
where
    T: GeoFloat,
{
    sweep_points
        .into_iter()
        .map(|sweep_point| {
            Point(Coord {
                x: sweep_point.x,
                y: sweep_point.y,
            })
        })
        .collect()
}
