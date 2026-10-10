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

/// A registration index scoped to the topology that returned it.
/// IDs from another topology must not be reused, even if their indices match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LayerId(pub(crate) usize);
impl LayerId {
    /// Return the zero-based layer registration index.
    #[must_use]
    pub fn index(self) -> usize {
        self.0
    }
}

/// Feature provenance within a topology, including an optional original part index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FeatureRef {
    /// Topology-scoped layer ID.
    pub layer: LayerId,
    /// Signed feature ID, unique within its layer.
    pub fid: i64,
    /// Zero-based depth-first part index, including skipped degenerate parts.
    /// `None` means multiple parts contributed or no single part is identified.
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

/// Broad input geometry families used to check rule compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GeometryKind {
    /// Points and multipoints.
    Point,
    /// Segments, line strings and multiline strings.
    Line,
    /// Polygons, multipolygons, rectangles and triangles.
    Polygon,
    /// Geometry collections, whose children are checked recursively for rules.
    Collection,
    /// Any geometry family, accepted by the multipart and containment rules.
    Any,
}
impl GeometryKind {
    /// Classify a geometry without validating its coordinates or collection children.
    #[must_use]
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

/// An owned planar layer. Cached views cannot be invalidated through mutation.
///
/// Construction defers finite-coordinate and duplicate-FID checks until rule
/// registration, warnings or validation. Cleaning removes consecutive duplicate
/// coordinates and skips lines with fewer than two coordinates and polygon rings
/// with fewer than four coordinates or three distinct vertices. A degenerate
/// exterior skips the polygon; degenerate holes are skipped individually.
/// Part indices retain the original traversal positions. Coordinate units are
/// supplied by the caller; the core does not interpret or transform a CRS.
#[derive(Debug)]
pub struct Layer {
    name: String,
    fids: Vec<i64>,
    geometries: Vec<Geometry>,
    parts: OnceLock<Result<Parts, Error>>,
}
impl Layer {
    /// Own geometries in input order, assigning signed FIDs starting at zero.
    /// Validation is deferred until this layer's parts are requested.
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
    /// Own features in input order, preserving signed FIDs and original geometries.
    /// FIDs must be unique within this layer; validation is deferred.
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
    /// Return the caller-supplied layer name, which need not be unique.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Iterate original, uncleaned features in input order.
    pub fn features(&self) -> impl ExactSizeIterator<Item = (i64, &Geometry)> {
        self.fids.iter().copied().zip(&self.geometries)
    }
    /// Return the number of original features, including degenerate geometries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fids.len()
    }
    /// Return whether the layer contains no original features.
    #[must_use]
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
    pub union: OnceLock<crate::engine::polygons::Union>,
    pub cross: OnceLock<crate::engine::cross::Views>,
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
    fn next_reference(mut reference: FeatureRef, part: &mut u32) -> FeatureRef {
        reference.part = Some(*part);
        *part += 1;
        reference
    }

    fn add_point(&mut self, p: &Point, reference: FeatureRef, part: &mut u32) {
        self.points.push(Part {
            reference: Self::next_reference(reference, part),
            geometry: *p,
        });
    }

    fn add_line(&mut self, l: &LineString, reference: FeatureRef, part: &mut u32) {
        let reference = Self::next_reference(reference, part);
        let geometry = clean_line(l);
        if geometry.0.len() < 2 {
            self.skipped += 1;
        } else {
            self.lines.push(Part {
                reference,
                geometry,
            });
        }
    }

    fn add_polygon(&mut self, p: &Polygon, reference: FeatureRef, part: &mut u32) {
        let reference = Self::next_reference(reference, part);
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

    fn add(&mut self, g: &Geometry, reference: FeatureRef, part: &mut u32) {
        match g {
            Geometry::MultiPoint(m) => {
                for p in m {
                    self.add_point(p, reference, part);
                }
            }
            Geometry::MultiLineString(m) => {
                for p in m {
                    self.add_line(p, reference, part);
                }
            }
            Geometry::MultiPolygon(m) => {
                for p in m {
                    self.add_polygon(p, reference, part);
                }
            }
            Geometry::GeometryCollection(m) => {
                for p in m {
                    self.add(p, reference, part);
                }
            }
            Geometry::Line(l) => self.add_line(&LineString(vec![l.start, l.end]), reference, part),
            Geometry::Rect(r) => self.add_polygon(&r.to_polygon(), reference, part),
            Geometry::Triangle(t) => self.add_polygon(&t.to_polygon(), reference, part),
            Geometry::Point(p) => self.add_point(p, reference, part),
            Geometry::LineString(l) => self.add_line(l, reference, part),
            Geometry::Polygon(p) => self.add_polygon(p, reference, part),
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

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{GeometryCollection, MultiLineString, MultiPolygon, line_string, polygon};

    #[test]
    fn multipart_cleaning_keeps_original_positions_and_counts_skipped_rings() {
        let line = line_string![(x:0.,y:0.),(x:0.,y:0.),(x:1.,y:0.)];
        let mut polygon = polygon![(x:0.,y:0.),(x:1.,y:0.),(x:1.,y:1.),(x:0.,y:0.)];
        polygon.interiors_push(LineString::from(vec![(0.5, 0.5); 4]));
        let geometry = Geometry::GeometryCollection(GeometryCollection(vec![
            MultiLineString(vec![LineString::empty(), line.clone()]).into(),
            MultiPolygon(vec![Polygon::empty(), polygon.clone()]).into(),
            Point::new(4., 5.).into(),
        ]));
        let layer = Layer::from_features("parts", [(-12, geometry.clone())]);
        let parts = layer.parts(LayerId(3)).unwrap();
        assert_eq!(parts.skipped, 3);
        assert_eq!(
            parts.lines[0].geometry,
            line_string![(x:0.,y:0.),(x:1.,y:0.)]
        );
        assert_eq!(parts.polygons[0].geometry.exterior(), polygon.exterior());
        assert!(parts.polygons[0].geometry.interiors().is_empty());
        for (reference, part) in [
            (parts.lines[0].reference, 1),
            (parts.polygons[0].reference, 3),
            (parts.points[0].reference, 4),
        ] {
            assert_eq!(
                reference,
                FeatureRef {
                    layer: LayerId(3),
                    fid: -12,
                    part: Some(part)
                }
            );
        }
        assert_eq!(layer.features().next().unwrap().1, &geometry);
    }

    #[test]
    fn multipart_nonfinite_coordinates_are_rejected_before_skipping_degeneracies() {
        let layer = Layer::from_features(
            "bad",
            [(
                42,
                MultiLineString(vec![LineString::from(vec![(f64::NAN, 0.)])]).into(),
            )],
        );
        assert!(matches!(
            layer.parts(LayerId(0)),
            Err(Error::NonFinite { fid: 42, .. })
        ));
    }
}
