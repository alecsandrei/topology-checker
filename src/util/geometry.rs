use geo::{
    algorithm::LineIntersection,
    sweep::{Intersections, SweepPoint},
    Coord, GeoFloat, Geometry, Line, LineString, LinesIter, MultiPolygon, Point, Polygon,
};
use itertools::{Either, Itertools};
use rayon::{iter::ParallelIterator, prelude::*};
use std::collections::BTreeSet;

/// Convert Geometry to Linestring.
/// Converts multipart features to singlepart.
pub fn flatten_linestrings<T: GeoFloat + Send + Sync>(
    geometries: Vec<Geometry<T>>,
) -> Vec<LineString<T>> {
    geometries
        .into_iter()
        .par_bridge()
        .flat_map_iter(|geometry| match geometry {
            Geometry::LineString(linestring) => {
                Box::new(std::iter::once(linestring)) as Box<dyn Iterator<Item = LineString<T>>>
            }
            Geometry::MultiLineString(multilinestring) => {
                Box::new(multilinestring.into_iter()) as Box<dyn Iterator<Item = LineString<T>>>
            }
            Geometry::Line(line) => {
                Box::new(std::iter::once(line.into())) as Box<dyn Iterator<Item = LineString<T>>>
            }
            _ => panic!("Unallowed geometries found."),
        })
        .collect()
}

pub fn flatten_points<T: GeoFloat + Send + Sync>(geometries: Vec<Geometry<T>>) -> Vec<Point<T>> {
    geometries
        .into_iter()
        .par_bridge()
        .flat_map_iter(|geometry| match geometry {
            Geometry::Point(point) => {
                Box::new(std::iter::once(point)) as Box<dyn Iterator<Item = Point<T>>>
            }
            Geometry::MultiPoint(points) => {
                Box::new(points.into_iter()) as Box<dyn Iterator<Item = Point<T>>>
            }
            _ => panic!("Unallowed geometries found."),
        })
        .collect()
}

pub fn is_polygon<T: GeoFloat>(geometry: &Geometry<T>) -> bool {
    if let Geometry::Polygon(_) = geometry {
        return true;
    } else if let Geometry::MultiPolygon(_) = geometry {
        return true;
    }
    false
}

pub fn is_point<T: GeoFloat>(geometry: &Geometry<T>) -> bool {
    if let Geometry::MultiPoint(_) = geometry {
        return true;
    } else if let Geometry::Point(_) = geometry {
        return true;
    }
    false
}

pub fn is_line<T: GeoFloat>(geometry: &Geometry<T>) -> bool {
    if let Geometry::LineString(_) = geometry {
        return true;
    } else if let Geometry::MultiLineString(_) = geometry {
        return true;
    } else if let Geometry::Line(_) = geometry {
        return true;
    }
    false
}

/// Convert Geometry to Polygon.
/// Converts multipart features to singlepart.
pub fn flatten_polygons(geometries: Vec<Geometry>) -> Vec<Polygon> {
    geometries
        .into_iter()
        .par_bridge()
        .flat_map_iter(|geometry| match geometry {
            Geometry::Polygon(polygon) => {
                Box::new(std::iter::once(polygon)) as Box<dyn Iterator<Item = Polygon>>
            }
            Geometry::MultiPolygon(multipolygon) => {
                Box::new(multipolygon.into_iter()) as Box<dyn Iterator<Item = Polygon>>
            }
            _ => panic!("Unallowed geometries found."),
        })
        .collect()
}

pub struct PartitionedPolygons<T: GeoFloat + Send + Sync>(pub Vec<Polygon<T>>, pub Vec<MultiPolygon<T>>);

impl<T> PartitionedPolygons<T>
where
    T: GeoFloat + Send + Sync,
{
    fn from_tuple(polygons: (Vec<Polygon<T>>, Vec<MultiPolygon<T>>)) -> Self {
        PartitionedPolygons(polygons.0, polygons.1)
    }
    pub fn from_geometries(geometries: Vec<Geometry<T>>) -> PartitionedPolygons<T> {
        PartitionedPolygons::from_tuple(geometries.into_iter().partition_map(|geometry| {
            match geometry {
                Geometry::Polygon(polygon) => Either::Left(polygon),
                Geometry::MultiPolygon(multipolygon) => Either::Right(multipolygon),
                _ => panic!("Unallowed geometries found."),
            }
        }))
    }
}

impl<T> IntoIterator for PartitionedPolygons<T>
where
    T: GeoFloat + Send + Sync,
{
    type Item = Polygon<T>;
    type IntoIter = std::iter::Chain<
        std::vec::IntoIter<Self::Item>,
        std::iter::Flatten<std::vec::IntoIter<geo::MultiPolygon<T>>>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter().chain(self.1.into_iter().flatten())
    }
}

/// Converts Linestring to Line.
pub fn explode_linestrings<T: GeoFloat + Send + Sync>(
    linestrings: &Vec<LineString<T>>,
) -> Vec<Line<T>> {
    linestrings
        .iter()
        .par_bridge()
        .flat_map_iter(|linestring| linestring.lines_iter())
        .collect()
}

/// Extract inner points (points that are not endpoints) from linestrings.
pub fn linestring_inner_points<T: GeoFloat>(linestring: &Vec<LineString<T>>) -> Vec<SweepPoint<T>> {
    let mut vec: Vec<SweepPoint<T>> = Vec::new();
    for line in linestring.into_iter() {
        for coord in &line.0[1..line.0.len() - 1] {
            let point: SweepPoint<T> = <Coord<T> as Into<SweepPoint<T>>>::into(*coord);
            vec.push(point);
        }
    }
    vec
}

/// Extract endpoints from linestrings.
pub fn linestring_endpoints<T>(linestring: &Vec<LineString<T>>) -> Vec<SweepPoint<T>>
where
    T: GeoFloat,
    Coord<T>: From<Point<T>>,
{
    let mut vec: Vec<SweepPoint<T>> = Vec::new();
    for line in linestring.into_iter() {
        let mut points = line.points();
        vec.push(<Point<T> as Into<SweepPoint<T>>>::into(
            points.next().unwrap(),
        ));
        vec.push(<Point<T> as Into<SweepPoint<T>>>::into(
            points.next_back().unwrap(),
        ));
    }
    vec
}

// Extract single point and line intersections from lines.
// Returns a tuple containing collinear lines and a tuple of
// unique proper single points and unique improper single points.
pub fn intersections<T, L, R>(
    lines: impl IntoIterator<Item = Line<T>>,
) -> (
    Vec<Line<T>>,
    (BTreeSet<SweepPoint<T>>, BTreeSet<SweepPoint<T>>),
)
where
    T: GeoFloat,
    L: From<geo::Coord<T>>,
    R: From<geo::Coord<T>>,
    BTreeSet<SweepPoint<T>>: Extend<R>,
    BTreeSet<SweepPoint<T>>: Extend<L>,
{
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

/// Converts [Coord] to [Point]
pub fn coords_to_points<T>(coords: impl IntoIterator<Item = Coord<T>>) -> Vec<Point>
where
    T: GeoFloat,
    Point: From<Coord<T>>,
{
    coords.into_iter().map_into().collect()
}

/// Converts [SweepPoint] to [Point].
pub fn sweep_points_to_points<T>(
    sweep_points: impl IntoIterator<Item = SweepPoint<T>>,
) -> Vec<Point<T>>
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
