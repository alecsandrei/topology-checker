//! Immutable views shared by cross-geometry rules.
use crate::{
    FeatureRef,
    engine::index::{self, Index},
    model::{Part, Parts},
};
use geo::{
    BoundingRect, Geometry, GeometryCollection, Line, LineString, MultiLineString, MultiPoint,
    MultiPolygon, Point, Rect, unary_union,
};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct Feature {
    pub reference: FeatureRef,
    pub geometry: Geometry,
    pub points: Vec<Point>,
    pub area: MultiPolygon,
}
#[derive(Debug, Default)]
pub(crate) struct Views {
    pub features: Vec<Feature>,
    pub feature_index: Index,
    pub endpoints: Vec<Part<Point>>,
    pub endpoint_index: Index,
    pub boundaries: Vec<Part<LineString>>,
    pub boundary_segments: Vec<Part<Line>>,
    pub boundary_index: Index,
    pub segments: Vec<Part<Line>>,
    pub segment_index: Index,
}

/// Preserve every dimension, including isolated contacts alongside line pieces.
pub(crate) fn combine(geometries: impl IntoIterator<Item = Geometry>) -> Geometry {
    fn add(
        g: Geometry,
        points: &mut Vec<Point>,
        lines: &mut Vec<LineString>,
        polygons: &mut Vec<geo::Polygon>,
    ) {
        match g {
            Geometry::Point(g) => points.push(g),
            Geometry::MultiPoint(g) => points.extend(g.0),
            Geometry::Line(g) => lines.push(g.into()),
            Geometry::LineString(g) => lines.push(g),
            Geometry::MultiLineString(g) => lines.extend(g.0),
            Geometry::Polygon(g) => polygons.push(g),
            Geometry::MultiPolygon(g) => polygons.extend(g.0),
            Geometry::Rect(g) => polygons.push(g.to_polygon()),
            Geometry::Triangle(g) => polygons.push(g.to_polygon()),
            Geometry::GeometryCollection(g) => {
                for child in g {
                    add(child, points, lines, polygons);
                }
            }
        }
    }
    let (mut points, mut lines, mut polygons) = (Vec::new(), Vec::new(), Vec::new());
    for g in geometries {
        add(g, &mut points, &mut lines, &mut polygons);
    }
    points.sort_by(|a, b| a.x().total_cmp(&b.x()).then(a.y().total_cmp(&b.y())));
    points.dedup();
    let mut result = Vec::new();
    if !points.is_empty() {
        result.push(if points.len() == 1 {
            points[0].into()
        } else {
            MultiPoint(points).into()
        });
    }
    if !lines.is_empty() {
        result.push(if lines.len() == 1 {
            lines.remove(0).into()
        } else {
            MultiLineString(lines).into()
        });
    }
    if !polygons.is_empty() {
        result.push(if polygons.len() == 1 {
            polygons.remove(0).into()
        } else {
            MultiPolygon(polygons).into()
        });
    }
    if result.len() == 1 {
        result.remove(0)
    } else {
        Geometry::GeometryCollection(GeometryCollection(result))
    }
}

pub(crate) fn views(parts: &Parts) -> &Views {
    parts.cross.get_or_init(|| {
        let mut views = Views::default();
        let mut features: BTreeMap<_, Vec<Geometry>> = BTreeMap::new();
        for p in &parts.points {
            features
                .entry(p.reference.feature())
                .or_default()
                .push(p.geometry.into());
        }
        for p in &parts.lines {
            features
                .entry(p.reference.feature())
                .or_default()
                .push(p.geometry.clone().into());
            let ends = [p.geometry.0[0], *p.geometry.0.last().expect("clean line")];
            if ends[0] != ends[1] {
                views.endpoints.extend(ends.map(|c| Part {
                    reference: p.reference,
                    geometry: Point(c),
                }));
            }
            views
                .segments
                .extend(p.geometry.lines().map(|geometry| Part {
                    reference: p.reference,
                    geometry,
                }));
        }
        for p in &parts.polygons {
            features
                .entry(p.reference.feature())
                .or_default()
                .push(p.geometry.clone().into());
            for ring in std::iter::once(p.geometry.exterior()).chain(p.geometry.interiors()) {
                views.boundaries.push(Part {
                    reference: p.reference,
                    geometry: ring.clone(),
                });
                views
                    .boundary_segments
                    .extend(ring.lines().map(|geometry| Part {
                        reference: p.reference,
                        geometry,
                    }));
            }
        }
        views.features = features
            .into_iter()
            .map(|((layer, fid), geometries)| {
                let points = geometries
                    .iter()
                    .filter_map(|g| {
                        if let Geometry::Point(p) = g {
                            Some(*p)
                        } else {
                            None
                        }
                    })
                    .collect();
                let polygons = geometries
                    .iter()
                    .filter_map(|g| {
                        if let Geometry::Polygon(p) = g {
                            Some(geo::orient::Orient::orient(
                                p,
                                geo::orient::Direction::Default,
                            ))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                Feature {
                    reference: FeatureRef {
                        layer,
                        fid,
                        part: None,
                    },
                    geometry: combine(geometries),
                    points,
                    area: unary_union(&polygons),
                }
            })
            .collect();
        views.feature_index = index::build(
            views
                .features
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.geometry.bounding_rect().map(|r| (i, r))),
        );
        views.endpoint_index = index::build(
            views
                .endpoints
                .iter()
                .enumerate()
                .map(|(i, p)| (i, Rect::new(p.geometry.0, p.geometry.0))),
        );
        views.boundary_index = index::build(
            views
                .boundary_segments
                .iter()
                .enumerate()
                .map(|(i, p)| (i, p.geometry.bounding_rect())),
        );
        views.segment_index = index::build(
            views
                .segments
                .iter()
                .enumerate()
                .map(|(i, p)| (i, p.geometry.bounding_rect())),
        );
        views
    })
}
