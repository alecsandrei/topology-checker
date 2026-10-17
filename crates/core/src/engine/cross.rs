//! Immutable views shared by cross-geometry rules.
use crate::{
    aggregate::Merge,
    engine::index::{self, Index},
    model::{Part, PartRef, Parts},
};
use geo::{BoundingRect, Geometry, Line, LineString, Point, Rect};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct Feature {
    pub reference: PartRef,
    pub geometry: Geometry,
    pub points: Vec<Point>,
    pub locations: Vec<super::location::Location>,
    pub area: Option<super::precision::Geometry>,
}
#[derive(Debug, Default)]
pub(crate) struct Views {
    pub features: Vec<Feature>,
    pub feature_index: Index,
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
                .push(p.geometry.as_ref().clone().into());
        }
        for p in &parts.polygons {
            features
                .entry(p.reference.fid)
                .or_default()
                .push(p.geometry.as_ref().clone().into());
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
                let locations = polygons
                    .iter()
                    .map(super::location::Location::new)
                    .collect();
                let area = (polygons.len() > 1)
                    .then(|| super::arrangement::overlay(&polygons, std::iter::empty(), false));
                let geometry = if area.is_some() {
                    Geometry::GeometryCollection(geo::GeometryCollection(geometries))
                } else {
                    Merge::AllDimensions.geometry(geometries)
                };
                Feature {
                    area,
                    locations,
                    reference: PartRef { fid, part: None },
                    geometry,
                    points,
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
        views
    })
}

/// Build only polygon boundaries, borrowing unchanged original rings.
pub(crate) fn boundaries(parts: &Parts) -> &[Part<super::storage::Shared<LineString>>] {
    parts.boundaries.get_or_init(|| {
        parts
            .polygons
            .iter()
            .flat_map(|p| {
                (0..=p.geometry.interiors().len()).map(|ring| Part {
                    reference: p.reference,
                    geometry: p.geometry.boundary(ring),
                    rings: Vec::new(),
                })
            })
            .collect()
    })
}
#[derive(Debug)]
pub(crate) struct Endpoints {
    pub points: Vec<Part<Point>>,
    pub index: Index,
}
impl Endpoints {
    /// Consecutive pairs retain the original ends of each exactly open part.
    /// Closure belongs to this query's tolerance, never to the structural cache.
    pub fn is_open(&self, i: usize, tolerance: f64) -> bool {
        !super::proximity::coincident(
            self.points[i].geometry.0,
            self.points[i ^ 1].geometry.0,
            tolerance,
        )
    }
}
pub(crate) fn endpoints(parts: &Parts) -> &Endpoints {
    parts.endpoints.get_or_init(|| {
        let points = parts
            .lines
            .iter()
            .filter(|p| p.geometry.0.first() != p.geometry.0.last())
            .flat_map(|p| {
                [p.geometry.0[0], *p.geometry.0.last().expect("clean line")].map(|c| Part {
                    reference: p.reference,
                    geometry: Point(c),
                    rings: Vec::new(),
                })
            })
            .collect::<Vec<_>>();
        let index = index::build(
            points
                .iter()
                .enumerate()
                .map(|(i, p)| (i, Rect::new(p.geometry.0, p.geometry.0))),
        );
        Endpoints { points, index }
    })
}
#[derive(Debug)]
pub(crate) struct Segments {
    pub segments: Vec<Part<Line>>,
    pub index: Index,
}
pub(crate) fn segments(parts: &Parts) -> &Segments {
    parts.segments.get_or_init(|| {
        let segments = parts
            .lines
            .iter()
            .flat_map(|p| {
                p.geometry.lines().map(|geometry| Part {
                    reference: p.reference,
                    geometry,
                    rings: Vec::new(),
                })
            })
            .collect::<Vec<_>>();
        let index = index::build(
            segments
                .iter()
                .enumerate()
                .map(|(i, p)| (i, p.geometry.bounding_rect())),
        );
        Segments { segments, index }
    })
}

impl Feature {
    pub fn is_areal(&self) -> bool {
        match &self.geometry {
            Geometry::Polygon(_) | Geometry::MultiPolygon(_) => true,
            Geometry::GeometryCollection(parts) => {
                !parts.0.is_empty() && parts.iter().all(|g| matches!(g, Geometry::Polygon(_)))
            }
            _ => false,
        }
    }
    pub fn output(&self) -> Result<Geometry, crate::Error> {
        let Some(area) = &self.area else {
            return Ok(self.geometry.clone());
        };
        // Several polygon parts are kept as a collection next to any other parts.
        let parts = match &self.geometry {
            Geometry::GeometryCollection(parts) => parts.0.as_slice(),
            single => std::slice::from_ref(single),
        };
        let mut geometries = parts
            .iter()
            .filter(|g| !matches!(g, Geometry::Polygon(_)))
            .cloned()
            .collect::<Vec<_>>();
        geometries.push(area.emit(0.)?.into());
        Ok(Merge::AllDimensions.geometry(geometries))
    }

    pub fn position(&self, p: geo::Coord) -> i8 {
        self.area.as_ref().map_or_else(
            || {
                self.locations
                    .iter()
                    .map(|l| l.position(p))
                    .max()
                    .unwrap_or(-1)
            },
            |a| a.position(p),
        )
    }
    pub fn near(&self, p: geo::Coord, tolerance: f64) -> bool {
        self.area.as_ref().map_or_else(
            || self.locations.iter().any(|l| l.near(p, tolerance)),
            |a| a.near(p, tolerance),
        )
    }
}
