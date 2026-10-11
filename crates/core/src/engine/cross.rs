//! Immutable views shared by cross-geometry rules.
use crate::{
    aggregate::Merge,
    engine::index::{self, Index},
    model::{Part, PartRef, Parts},
};
use geo::{BoundingRect, Geometry, Line, LineString, MultiPolygon, Point, Rect, unary_union};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct Feature {
    pub reference: PartRef,
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

pub(crate) fn views(parts: &Parts) -> &Views {
    parts.cross.get_or_init(|| {
        let mut views = Views::default();
        let mut features: BTreeMap<_, Vec<Geometry>> = BTreeMap::new();
        for p in &parts.points {
            features
                .entry(p.reference.fid)
                .or_default()
                .push(p.geometry.into());
        }
        for p in &parts.lines {
            features
                .entry(p.reference.fid)
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
                .entry(p.reference.fid)
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
            .map(|(fid, geometries)| {
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
                    reference: PartRef { fid, part: None },
                    geometry: Merge::AllDimensions.geometry(geometries),
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
