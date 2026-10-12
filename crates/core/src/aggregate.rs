//! Merge part-level violations into one violation per rule, kind and feature pair.
use crate::{FeatureRef, LayerId, Rule, Violation, ViolationKind};
use geo::{
    Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point,
    Polygon,
};
use std::collections::BTreeMap;

/// How the geometries of merged violations are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Merge {
    /// Keep only the highest dimension present, such as overlap areas over their edges.
    /// An unsupported first geometry, such as an empty multi geometry, is kept when
    /// nothing else contributes.
    #[cfg(test)]
    HighestDimension,
    /// Keep every dimension, such as isolated contacts alongside line pieces, in a
    /// collection when several are present.
    AllDimensions,
}
impl Merge {
    /// Combine geometries. Coincident points are deduplicated.
    pub(crate) fn geometry(self, geometries: impl IntoIterator<Item = Geometry>) -> Geometry {
        let mut collected = Collected::default();
        match self {
            #[cfg(test)]
            Self::HighestDimension => {
                let mut fallback = None;
                for (i, g) in geometries.into_iter().enumerate() {
                    if let Some(unsupported) = collected.simple(g)
                        && i == 0
                    {
                        fallback = Some(unsupported);
                    }
                }
                let [points, lines, polygons] = collected.dimensions();
                polygons
                    .or(lines)
                    .or(points)
                    .or(fallback)
                    .unwrap_or_else(|| MultiPoint(Vec::new()).into())
            }
            Self::AllDimensions => {
                for g in geometries {
                    collected.all(g);
                }
                let mut present = collected
                    .dimensions()
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                if present.len() == 1 {
                    present.remove(0)
                } else {
                    Geometry::GeometryCollection(GeometryCollection(present))
                }
            }
        }
    }
}

/// Points, lines and polygons gathered from several geometries.
#[derive(Default)]
struct Collected {
    points: Vec<Point>,
    lines: Vec<LineString>,
    polygons: Vec<Polygon>,
}
impl Collected {
    /// Collect a nonempty simple or multi geometry, returning any other geometry unchanged.
    fn simple(&mut self, g: Geometry) -> Option<Geometry> {
        match g {
            Geometry::Point(g) => self.points.push(g),
            Geometry::MultiPoint(g) if !g.0.is_empty() => self.points.extend(g.0),
            Geometry::Line(g) => self.lines.push(g.into()),
            Geometry::LineString(g) => self.lines.push(g),
            Geometry::MultiLineString(g) if !g.0.is_empty() => self.lines.extend(g.0),
            Geometry::Polygon(g) => self.polygons.push(g),
            Geometry::MultiPolygon(g) if !g.0.is_empty() => self.polygons.extend(g.0),
            other => return Some(other),
        }
        None
    }
    /// Collect every part, converting rectangles and triangles and flattening collections.
    fn all(&mut self, g: Geometry) {
        match g {
            Geometry::Rect(g) => self.polygons.push(g.to_polygon()),
            Geometry::Triangle(g) => self.polygons.push(g.to_polygon()),
            Geometry::GeometryCollection(g) => {
                for child in g {
                    self.all(child);
                }
            }
            // Empty multi geometries contribute nothing.
            g => {
                let _ = self.simple(g);
            }
        }
    }
    /// Return deduplicated points, then lines, then polygons, each when present.
    fn dimensions(mut self) -> [Option<Geometry>; 3] {
        fn single_or_multi<T: Into<Geometry>, M: Into<Geometry>>(
            mut parts: Vec<T>,
            multi: fn(Vec<T>) -> M,
        ) -> Option<Geometry> {
            match parts.len() {
                0 => None,
                1 => Some(parts.remove(0).into()),
                _ => Some(multi(parts).into()),
            }
        }
        // Polygon rule aggregation occurs on original operands before emission.
        // Never feed emitted intersection coordinates into another overlay.
        // Parallel candidate discovery must not change multipart ordering.
        self.lines.sort_by(|a, b| {
            a.0.iter()
                .zip(&b.0)
                .map(|(a, b)| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)))
                .find(|order| !order.is_eq())
                .unwrap_or_else(|| a.0.len().cmp(&b.0.len()))
        });
        self.points
            .sort_by(|a, b| a.x().total_cmp(&b.x()).then(a.y().total_cmp(&b.y())));
        self.points.dedup();
        [
            single_or_multi(self.points, MultiPoint),
            single_or_multi(self.lines, MultiLineString),
            single_or_multi(self.polygons, MultiPolygon),
        ]
    }
}

/// Merge violations sharing a rule, kind and origin and destination features.
/// Part indices are cleared when several parts contributed; groups follow key order.
pub(crate) fn aggregate(violations: Vec<Violation>, merge: Merge) -> Vec<Violation> {
    type Key = (
        Rule,
        ViolationKind,
        Option<(LayerId, i64)>,
        Option<(LayerId, i64)>,
    );
    let mut groups: BTreeMap<Key, Vec<Violation>> = BTreeMap::new();
    for v in violations {
        groups
            .entry((
                v.rule,
                v.kind,
                v.origin.map(FeatureRef::feature),
                v.destination.map(FeatureRef::feature),
            ))
            .or_default()
            .push(v);
    }
    groups
        .into_values()
        .map(|group| {
            let mut group = group.into_iter();
            let mut first = group.next().expect("aggregation groups are nonempty");
            let mut geometries = vec![std::mem::replace(
                &mut first.geometry,
                Geometry::GeometryCollection(GeometryCollection(Vec::new())),
            )];
            for v in group {
                if let (Some(a), Some(b)) = (&mut first.origin, v.origin) {
                    a.aggregate(b);
                }
                if let (Some(a), Some(b)) = (&mut first.destination, v.destination) {
                    a.aggregate(b);
                }
                geometries.push(v.geometry);
            }
            first.geometry = merge.geometry(geometries);
            first
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{Rect, coord};

    fn violation(geometry: Geometry, part: u32) -> Violation {
        Violation {
            rule: Rule::LineMustNotIntersect,
            kind: ViolationKind::Crossing,
            geometry,
            origin: Some(FeatureRef {
                layer: LayerId(0),
                fid: -4,
                part: Some(part),
            }),
            destination: Some(FeatureRef {
                layer: LayerId(1),
                fid: 8,
                part: Some(2),
            }),
        }
    }

    #[test]
    fn aggregation_retains_provenance_deduplication_and_geometry_priority() {
        let point = Point::new(1., 0.);
        let points = aggregate(
            vec![violation(point.into(), 0), violation(point.into(), 1)],
            Merge::HighestDimension,
        );
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].geometry, Geometry::Point(point));
        assert_eq!(points[0].origin.unwrap().part, None);
        assert_eq!(points[0].destination.unwrap().part, Some(2));
        let line = LineString::from(vec![(0., 0.), (1., 0.)]);
        let polygon = Rect::new(coord! {x:0.,y:0.}, coord! {x:1.,y:1.}).to_polygon();
        let mixed = aggregate(
            vec![
                violation(point.into(), 0),
                violation(line.into(), 0),
                violation(polygon.clone().into(), 1),
            ],
            Merge::HighestDimension,
        );
        assert_eq!(mixed[0].geometry, Geometry::Polygon(polygon));
        let empty = Geometry::MultiLineString(MultiLineString(Vec::new()));
        assert_eq!(
            aggregate(vec![violation(empty.clone(), 0)], Merge::HighestDimension)[0].geometry,
            empty
        );
    }

    #[test]
    fn all_dimensions_keep_contacts_beside_pieces_and_flatten_collections() {
        let point = Point::new(1., 0.);
        let line = LineString::from(vec![(0., 0.), (1., 0.)]);
        let rect = Rect::new(coord! {x:0.,y:0.}, coord! {x:1.,y:1.});
        let merged = aggregate(
            vec![
                violation(point.into(), 0),
                violation(
                    Geometry::GeometryCollection(GeometryCollection(vec![
                        point.into(),
                        line.clone().into(),
                    ])),
                    1,
                ),
                violation(rect.into(), 1),
            ],
            Merge::AllDimensions,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].geometry,
            Geometry::GeometryCollection(GeometryCollection(vec![
                point.into(),
                line.clone().into(),
                rect.to_polygon().into()
            ]))
        );
        assert_eq!(
            Merge::AllDimensions.geometry([line.clone().into()]),
            Geometry::LineString(line)
        );
        assert_eq!(
            Merge::AllDimensions.geometry([MultiPoint(Vec::new()).into()]),
            Geometry::GeometryCollection(GeometryCollection(Vec::new()))
        );
    }
}
