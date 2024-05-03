use geo::{
    algorithm::LineIntersection,
    sweep::{self, Intersections, SweepPoint},
    Coord, GeoFloat, Geometry, Line, LineString, LinesIter, Point,
};
use itertools::{Either, Itertools};
use rayon::{iter::ParallelIterator, prelude::*};

pub fn flatten_linestring(geometries: Vec<Geometry>) -> Vec<LineString<f64>> {
    geometries
        .into_iter()
        .par_bridge()
        .flat_map_iter(|geometry| match geometry {
            geo_types::Geometry::LineString(linestring) => {
                Box::new(std::iter::once(linestring)) as Box<dyn Iterator<Item = LineString<f64>>>
            }
            geo_types::Geometry::MultiLineString(multilinestring) => {
                Box::new(multilinestring.into_iter()) as Box<dyn Iterator<Item = LineString<f64>>>
            }
            _ => panic!("Unallowed geometries found."),
        })
        .collect()
}

pub fn flatten_lines(linestrings: Vec<LineString>) -> Vec<Line<f64>> {
    linestrings
        .iter()
        .par_bridge()
        .flat_map_iter(|linestring| linestring.lines_iter())
        .collect()
}

pub fn linestring_inner_points(linestring: &Vec<LineString<f64>>) -> Vec<SweepPoint<f64>> {
    // Provides an ordered vector of unique inner points (points that are not endpoints).
    let mut vec: Vec<SweepPoint<f64>> = Vec::new();
    for line in linestring.into_iter() {
        for coord in &line.0[1..line.0.len() - 1] {
            let point: SweepPoint<f64> = <Coord as Into<SweepPoint<f64>>>::into(*coord);
            vec.push(point);
        }
    }
    vec.sort();
    vec
}

pub fn linestring_endpoints(linestring: &Vec<LineString<f64>>) -> Vec<SweepPoint<f64>> {
    // Provides an ordered vector of unique endpoints (points that are not inner points).
    let mut vec: Vec<SweepPoint<f64>> = Vec::new();
    for line in linestring.into_iter() {
        let mut points = line.points();
        vec.push(<Point as Into<SweepPoint<f64>>>::into(
            points.next().unwrap(),
        ));
        vec.push(<Point as Into<SweepPoint<f64>>>::into(
            points.next_back().unwrap(),
        ));
    }
    vec.sort();
    vec
}

pub fn intersections(lines: Vec<Line<f64>>) -> (Vec<Line>, (Vec<Coord>, Vec<Coord>)) {
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

    let lines: Vec<Line> = lines
        .into_iter()
        .map(|line_intersection| {
            if let LineIntersection::Collinear { intersection } = line_intersection {
                intersection
            } else {
                unreachable!()
            }
        })
        .collect();

    let mut points: (Vec<_>, Vec<_>) =
        points
            .into_iter()
            .partition_map(|points_intersection| match points_intersection {
                LineIntersection::SinglePoint {
                    intersection,
                    is_proper: true,
                } => Either::Left(intersection),
                LineIntersection::SinglePoint {
                    intersection,
                    is_proper: false,
                } => Either::Right(intersection),
                _ => unreachable!(),
            });
    points.0.dedup();
    points.1.dedup();
    (lines, points)
}

pub fn coords_to_points(coords: Vec<Coord>) -> Vec<Point> {
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
