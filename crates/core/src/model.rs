use crate::{
    Error,
    engine::{
        cluster::bits,
        index::{self, Index},
    },
};
use geo::{BoundingRect, CoordsIter, Geometry, LineString, Point, Polygon, Rect};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LayerId(pub(crate) usize);
impl LayerId {
    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FeatureRef {
    pub layer: LayerId,
    pub fid: i64,
    pub part: Option<u32>,
}
impl FeatureRef {
    pub(crate) fn feature(self) -> (LayerId, i64) {
        (self.layer, self.fid)
    }
    pub(crate) fn aggregate(&mut self, other: Self) {
        if self.part != other.part {
            self.part = None;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GeometryKind {
    Point,
    Line,
    Polygon,
    Collection,
    Any,
}
impl GeometryKind {
    pub fn of(g: &Geometry) -> Self {
        match g {
            Geometry::Point(_) | Geometry::MultiPoint(_) => Self::Point,
            Geometry::Line(_) | Geometry::LineString(_) | Geometry::MultiLineString(_) => {
                Self::Line
            }
            Geometry::Polygon(_)
            | Geometry::MultiPolygon(_)
            | Geometry::Rect(_)
            | Geometry::Triangle(_) => Self::Polygon,
            Geometry::GeometryCollection(_) => Self::Collection,
        }
    }
}

/// An owned layer. Cached views cannot be invalidated through mutation.
#[derive(Debug)]
pub struct Layer {
    name: String,
    fids: Vec<i64>,
    geometries: Vec<Geometry>,
    parts: OnceLock<Result<Parts, Error>>,
}
impl Layer {
    pub fn from_geometries(
        name: impl Into<String>,
        geometries: impl IntoIterator<Item = Geometry>,
    ) -> Self {
        Self::from_features(
            name,
            geometries
                .into_iter()
                .enumerate()
                .map(|(i, g)| (i as i64, g)),
        )
    }
    pub fn from_features(
        name: impl Into<String>,
        features: impl IntoIterator<Item = (i64, Geometry)>,
    ) -> Self {
        let (fids, geometries) = features.into_iter().unzip();
        Self {
            name: name.into(),
            fids,
            geometries,
            parts: OnceLock::new(),
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn features(&self) -> impl ExactSizeIterator<Item = (i64, &Geometry)> {
        self.fids.iter().copied().zip(&self.geometries)
    }
    pub fn len(&self) -> usize {
        self.fids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.fids.is_empty()
    }
    pub(crate) fn check_kind(&self, expected: GeometryKind) -> Result<(), Error> {
        fn check(g: &Geometry, expected: GeometryKind) -> Option<GeometryKind> {
            if expected == GeometryKind::Any {
                return None;
            }
            if let Geometry::GeometryCollection(c) = g {
                return c.iter().find_map(|g| check(g, expected));
            }
            let found = GeometryKind::of(g);
            (found != expected).then_some(found)
        }
        for (fid, g) in self.features() {
            if let Some(found) = check(g, expected) {
                return Err(Error::GeometryKind {
                    layer: self.name.clone(),
                    fid,
                    expected,
                    found,
                });
            }
        }
        Ok(())
    }
    pub(crate) fn parts(&self, id: LayerId) -> Result<&Parts, Error> {
        self.parts
            .get_or_init(|| self.make_parts(id))
            .as_ref()
            .map_err(Clone::clone)
    }
    fn make_parts(&self, id: LayerId) -> Result<Parts, Error> {
        let mut out = Parts::default();
        let mut seen = HashSet::new();
        for (fid, g) in self.features() {
            if !seen.insert(fid) {
                return Err(Error::DuplicateFid {
                    layer: self.name.clone(),
                    fid,
                });
            }
            if g.coords_iter()
                .any(|c| !c.x.is_finite() || !c.y.is_finite())
            {
                return Err(Error::NonFinite {
                    layer: self.name.clone(),
                    fid,
                });
            }
            out.add(
                g,
                FeatureRef {
                    layer: id,
                    fid,
                    part: Some(0),
                },
                &mut 0,
            );
        }
        out.point_index = index::build(
            out.points
                .iter()
                .enumerate()
                .map(|(i, p)| (i, Rect::new(p.geometry.0, p.geometry.0))),
        );
        out.line_index = index::build(
            out.lines
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.geometry.bounding_rect().map(|r| (i, r))),
        );
        out.polygon_index = index::build(
            out.polygons
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.geometry.bounding_rect().map(|r| (i, r))),
        );
        Ok(out)
    }
}
#[derive(Debug)]
pub(crate) struct Part<G> {
    pub reference: FeatureRef,
    pub geometry: G,
}
#[derive(Debug, Default)]
pub(crate) struct Parts {
    pub points: Vec<Part<Point>>,
    pub lines: Vec<Part<LineString>>,
    pub polygons: Vec<Part<Polygon>>,
    pub point_index: Index,
    pub line_index: Index,
    pub polygon_index: Index,
    pub skipped: usize,
}
fn clean_line(line: &LineString) -> LineString {
    let mut coords = line.0.clone();
    coords.dedup();
    LineString(coords)
}
fn ring_valid(ring: &LineString) -> bool {
    ring.0.len() >= 4
        && ring
            .0
            .iter()
            .map(|c| (bits(c.x), bits(c.y)))
            .collect::<HashSet<_>>()
            .len()
            >= 3
}
impl Parts {
    fn add(&mut self, g: &Geometry, mut reference: FeatureRef, part: &mut u32) {
        match g {
            Geometry::MultiPoint(m) => {
                for p in m {
                    self.add(&Geometry::Point(*p), reference, part);
                }
            }
            Geometry::MultiLineString(m) => {
                for p in m {
                    self.add(&Geometry::LineString(p.clone()), reference, part);
                }
            }
            Geometry::MultiPolygon(m) => {
                for p in m {
                    self.add(&Geometry::Polygon(p.clone()), reference, part);
                }
            }
            Geometry::GeometryCollection(m) => {
                for p in m {
                    self.add(p, reference, part);
                }
            }
            Geometry::Line(l) => self.add(
                &Geometry::LineString(LineString(vec![l.start, l.end])),
                reference,
                part,
            ),
            Geometry::Rect(r) => self.add(&Geometry::Polygon(r.to_polygon()), reference, part),
            Geometry::Triangle(t) => self.add(&Geometry::Polygon(t.to_polygon()), reference, part),
            _ => {
                reference.part = Some(*part);
                *part += 1;
                match g {
                    Geometry::Point(p) => self.points.push(Part {
                        reference,
                        geometry: *p,
                    }),
                    Geometry::LineString(l) => {
                        let l = clean_line(l);
                        if l.0.len() < 2 {
                            self.skipped += 1;
                        } else {
                            self.lines.push(Part {
                                reference,
                                geometry: l,
                            });
                        }
                    }
                    Geometry::Polygon(p) => {
                        let exterior = clean_line(p.exterior());
                        if !ring_valid(&exterior) {
                            self.skipped += 1;
                            return;
                        }
                        let holes = p
                            .interiors()
                            .iter()
                            .map(clean_line)
                            .filter(|r| {
                                let valid = ring_valid(r);
                                if !valid {
                                    self.skipped += 1;
                                }
                                valid
                            })
                            .collect();
                        self.polygons.push(Part {
                            reference,
                            geometry: Polygon::new(exterior, holes),
                        });
                    }
                    _ => unreachable!("multipart and derived geometries are handled above"),
                }
            }
        }
    }
}
pub(crate) fn part_count(g: &Geometry) -> usize {
    match g {
        Geometry::MultiPoint(m) => m.len(),
        Geometry::MultiLineString(m) => m.0.len(),
        Geometry::MultiPolygon(m) => m.0.len(),
        Geometry::GeometryCollection(m) => m.iter().map(part_count).sum(),
        _ => 1,
    }
}
