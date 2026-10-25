use crate::cache::Cache;
use crate::{
    Error,
    engine::{
        index::{self, Entry, Packed},
        storage::Shared,
    },
};
use geo::{BoundingRect, CoordsIter, Geometry, LineString, Point, Polygon, Rect};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};

/// A registration index scoped to the topology that returned it.
/// IDs from another topology must not be reused, even if their indices match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[non_exhaustive]
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
    /// Create provenance using an identity from the relevant topology.
    #[must_use]
    pub fn new(layer: LayerId, fid: i64, part: Option<u32>) -> Self {
        Self { layer, fid, part }
    }

    pub(crate) fn feature(self) -> (LayerId, i64) {
        (self.layer, self.fid)
    }
    pub(crate) fn aggregate(&mut self, other: Self) {
        if self.part != other.part {
            self.part = None;
        }
    }
}

/// Layer-local part provenance, cached independently of any topology.
///
/// It deliberately has no equality or ordering: parts from different layers may
/// share FIDs, so callers attach a `LayerId` with [`PartRef::at`] before comparing
/// references that are not known to come from one layer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PartRef {
    pub fid: i64,
    pub part: Option<u32>,
}
impl PartRef {
    pub(crate) fn at(self, layer: LayerId) -> FeatureRef {
        FeatureRef {
            layer,
            fid: self.fid,
            part: self.part,
        }
    }
}

/// A cached layer validation failure, independent of the layer's registration ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Invalid {
    DuplicateFid(i64),
    NonFinite(i64),
    Polygon {
        fid: i64,
        part: u32,
        ring: Option<usize>,
        reason: &'static str,
    },
}
impl Invalid {
    /// The feature that failed validation.
    pub(crate) fn fid(&self) -> i64 {
        let (Self::DuplicateFid(fid) | Self::NonFinite(fid) | Self::Polygon { fid, .. }) = self;
        *fid
    }
    pub(crate) fn at(self, layer: LayerId, name: &str) -> Error {
        let name = name.to_string();
        match self {
            Self::DuplicateFid(fid) => Error::DuplicateFid { layer, name, fid },
            Self::NonFinite(fid) => Error::NonFinite { layer, name, fid },
            Self::Polygon {
                fid,
                part,
                ring,
                reason,
            } => Error::InvalidPolygon {
                layer,
                name,
                fid,
                part,
                ring,
                reason: reason.into(),
            },
        }
    }
}

/// Broad input geometry families used to check rule compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
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
    /// Return whether geometry of this family can be a rule input requiring `required`.
    /// Collections are classified by their children, so they never satisfy a family.
    #[must_use]
    pub fn satisfies(self, required: Self) -> bool {
        required == Self::Any || self == required
    }
}

/// An owned planar layer. Cached views cannot be invalidated through mutation.
///
/// Caches do not depend on a topology or `LayerId`, so one `Arc<Layer>` can be
/// registered in several topologies and its cleaned parts, indexes, polygon
/// union and cross-geometry views are built once and reused.
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
    geometries: Arc<Vec<Geometry>>,
    ranks: Cache<std::collections::BTreeMap<i64, usize>>,
    parts: Cache<Result<Parts, Invalid>>,
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
            geometries: Arc::new(geometries),
            ranks: Cache::new(),
            parts: Cache::new(),
        }
    }
    /// Return the caller-supplied layer name, which need not be unique.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Iterate original, uncleaned features in input order.
    pub fn features(&self) -> impl ExactSizeIterator<Item = (i64, &Geometry)> {
        self.fids.iter().copied().zip(self.geometries.iter())
    }
    pub(crate) fn ranks(&self) -> &std::collections::BTreeMap<i64, usize> {
        self.ranks.get_or_init(|| {
            self.fids
                .iter()
                .copied()
                .enumerate()
                .map(|(i, fid)| (fid, i))
                .collect()
        })
    }
    /// Return the original feature at a zero-based input position.
    #[must_use]
    pub fn feature(&self, index: usize) -> Option<(i64, &Geometry)> {
        Some((*self.fids.get(index)?, self.geometries.get(index)?))
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
    pub(crate) fn check_kind(&self, id: LayerId, expected: GeometryKind) -> Result<(), Error> {
        fn check(g: &Geometry, expected: GeometryKind) -> Option<GeometryKind> {
            if expected == GeometryKind::Any {
                return None;
            }
            if let Geometry::GeometryCollection(c) = g {
                return c.iter().find_map(|g| check(g, expected));
            }
            let found = GeometryKind::of(g);
            (!found.satisfies(expected)).then_some(found)
        }
        for (fid, g) in self.features() {
            if let Some(found) = check(g, expected) {
                return Err(Error::GeometryKind {
                    layer: id,
                    name: self.name.clone(),
                    fid,
                    expected,
                    found,
                });
            }
        }
        Ok(())
    }
    /// Validate cleaned geometry without registering a rule.
    ///
    /// # Errors
    /// Returns a deferred input error with the caller's layer identity.
    pub fn check(&self, id: LayerId) -> Result<(), Error> {
        self.parts().map(|_| ()).map_err(|e| e.at(id, self.name()))
    }
    pub(crate) fn parts(&self) -> Result<&Parts, Invalid> {
        self.parts
            .get_or_init(|| self.make_parts())
            .as_ref()
            .map_err(Clone::clone)
    }
    fn make_parts(&self) -> Result<Parts, Invalid> {
        let mut out = Parts {
            source: Some(Arc::clone(&self.geometries)),
            ..Parts::default()
        };
        let mut seen = HashSet::new();
        for (feature, (fid, g)) in self.features().enumerate() {
            if !seen.insert(fid) {
                return Err(Invalid::DuplicateFid(fid));
            }
            if g.coords_iter()
                .any(|c| !c.x.is_finite() || !c.y.is_finite())
            {
                return Err(Invalid::NonFinite(fid));
            }
            let first = out.polygons.len();
            out.add(
                g,
                PartRef { fid, part: Some(0) },
                &mut 0,
                Some(feature),
                &mut Vec::new(),
            );
            for p in &out.polygons[first..] {
                if let Some((ring, reason)) = crate::engine::validity::polygon(&p.geometry) {
                    return Err(Invalid::Polygon {
                        fid,
                        part: p.reference.part.unwrap_or(0),
                        ring: Some(p.rings[ring]),
                        reason,
                    });
                }
            }
            if let Some(invalid) = out.invalid.take() {
                return Err(invalid);
            }
        }
        Ok(out)
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Part<G> {
    pub reference: PartRef,
    pub geometry: G,
    pub rings: Vec<usize>,
}
#[derive(Debug, Default)]
pub(crate) struct Parts {
    pub points: Vec<Part<Point>>,
    pub lines: Vec<Part<Shared<LineString>>>,
    pub polygons: Vec<Part<Shared<Polygon>>>,
    point_index: Cache<Packed>,
    line_index: Cache<Packed>,
    polygon_index: Cache<Packed>,
    point_order: Cache<Vec<Entry>>,
    line_order: Cache<Vec<Entry>>,
    polygon_order: Cache<Vec<Entry>>,
    pub locations: Cache<Vec<crate::engine::location::Location>>,
    pub union: Cache<crate::engine::polygons::Union>,
    pub boundaries: Cache<Vec<Part<Shared<LineString>>>>,
    pub endpoints: Cache<crate::engine::cross::Endpoints>,
    pub segments: Cache<crate::engine::cross::Segments>,
    pub cross: Cache<crate::engine::cross::Views>,
    pub skipped: usize,
    invalid: Option<Invalid>,
    source: Option<Arc<Vec<Geometry>>>,
    ranges: Cache<std::collections::BTreeMap<i64, [std::ops::Range<usize>; 3]>>,
}
fn clean_line(line: &LineString) -> LineString {
    let mut coords = line.0.clone();
    coords.dedup();
    LineString(coords)
}
fn ring_valid(ring: &LineString) -> bool {
    if ring.0.len() < 4 {
        return false;
    }
    let first = ring.0[0];
    let Some(second) = ring.0.iter().find(|&&c| c != first) else {
        return false;
    };
    ring.0.iter().any(|c| *c != first && c != second)
}

impl Parts {
    fn feature_ranges(&self) -> &std::collections::BTreeMap<i64, [std::ops::Range<usize>; 3]> {
        self.ranges.get_or_init(|| {
            let mut map = std::collections::BTreeMap::new();
            fn add<G>(
                map: &mut std::collections::BTreeMap<i64, [std::ops::Range<usize>; 3]>,
                parts: &[Part<G>],
                family: usize,
            ) {
                for (i, p) in parts.iter().enumerate() {
                    let ranges = map
                        .entry(p.reference.fid)
                        .or_insert_with(|| [0..0, 0..0, 0..0]);
                    let range = &mut ranges[family];
                    if range.start == range.end {
                        range.start = i;
                    }
                    range.end = i + 1;
                }
            }
            add(&mut map, &self.points, 0);
            add(&mut map, &self.lines, 1);
            add(&mut map, &self.polygons, 2);
            map
        })
    }
    pub fn point_feature(&self, fid: i64) -> &[Part<Point>] {
        self.feature_ranges()
            .get(&fid)
            .map_or(&[], |r| &self.points[r[0].clone()])
    }
    pub fn feature(&self, fid: i64) -> Self {
        let ranges = self.feature_ranges();
        let empty = [0..0, 0..0, 0..0];
        let r = ranges.get(&fid).unwrap_or(&empty);
        Self {
            source: self.source.clone(),
            points: self.points[r[0].clone()].to_vec(),
            lines: self.lines[r[1].clone()].to_vec(),
            polygons: self.polygons[r[2].clone()].to_vec(),
            ..Self::default()
        }
    }
    pub fn point_index(&self) -> &Packed {
        self.point_index.get_or_init(|| {
            index::packed(self.points.len(), "index.points", |i| {
                let point = self.points[i].geometry.0;
                Some(Rect::new(point, point))
            })
        })
    }
    pub fn line_index(&self) -> &Packed {
        self.line_index.get_or_init(|| {
            index::packed(self.lines.len(), "index.lines", |i| {
                self.lines[i].geometry.bounding_rect()
            })
        })
    }
    pub fn polygon_index(&self) -> &Packed {
        self.polygon_index.get_or_init(|| {
            index::packed(self.polygons.len(), "index.polygons", |i| {
                self.polygons[i].geometry.bounding_rect()
            })
        })
    }
    /// The points in [`index::arranged`] order, to search another index with.
    pub fn point_order(&self) -> &[Entry] {
        self.point_order.get_or_init(|| {
            let point = |i: usize| {
                Some(Rect::new(
                    self.points[i].geometry.0,
                    self.points[i].geometry.0,
                ))
            };
            index::arranged(self.points.len(), point)
        })
    }
    /// The lines in [`index::arranged`] order, to search another index with.
    pub fn line_order(&self) -> &[Entry] {
        self.line_order.get_or_init(|| {
            index::arranged(self.lines.len(), |i| self.lines[i].geometry.bounding_rect())
        })
    }
    /// The polygons in [`index::arranged`] order, to search another index with.
    pub fn polygon_order(&self) -> &[Entry] {
        self.polygon_order.get_or_init(|| {
            index::arranged(self.polygons.len(), |i| {
                self.polygons[i].geometry.bounding_rect()
            })
        })
    }

    fn next_reference(mut reference: PartRef, part: &mut u32) -> PartRef {
        reference.part = Some(*part);
        *part += 1;
        reference
    }

    fn add_point(&mut self, p: &Point, reference: PartRef, part: &mut u32) {
        self.points.push(Part {
            reference: Self::next_reference(reference, part),
            geometry: *p,
            rings: Vec::new(),
        });
    }

    fn add_line(
        &mut self,
        l: &LineString,
        reference: PartRef,
        part: &mut u32,
        feature: Option<usize>,
        path: &[usize],
    ) {
        let reference = Self::next_reference(reference, part);
        let geometry = match feature {
            Some(feature) if !l.0.windows(2).any(|p| p[0] == p[1]) => Shared::Source {
                geometries: Arc::clone(self.source.as_ref().expect("layer storage")),
                feature,
                path: path.to_vec(),
            },
            _ => Shared::Cleaned(clean_line(l)),
        };
        if geometry.0.len() < 2 {
            self.skipped += 1;
        } else {
            self.lines.push(Part {
                reference,
                geometry,
                rings: Vec::new(),
            });
        }
    }

    fn add_polygon(
        &mut self,
        p: &Polygon,
        reference: PartRef,
        part: &mut u32,
        feature: Option<usize>,
        path: &[usize],
    ) {
        let reference = Self::next_reference(reference, part);
        if let Some(feature) = feature {
            let rings = std::iter::once(p.exterior())
                .chain(p.interiors())
                .collect::<Vec<_>>();
            if rings
                .iter()
                .all(|r| ring_valid(r) && !r.0.windows(2).any(|p| p[0] == p[1]))
            {
                self.polygons.push(Part {
                    reference,
                    geometry: Shared::Source {
                        geometries: Arc::clone(self.source.as_ref().expect("layer storage")),
                        feature,
                        path: path.to_vec(),
                    },
                    rings: (0..rings.len()).collect(),
                });
                return;
            }
        }
        let exterior = clean_line(p.exterior());
        if !ring_valid(&exterior) {
            self.skipped += 1;
            return;
        }
        let mut rings = vec![0];
        let holes = p
            .interiors()
            .iter()
            .enumerate()
            .map(|(i, r)| (i + 1, clean_line(r)))
            .filter_map(|(i, r)| {
                let valid = ring_valid(&r);
                if !valid {
                    self.skipped += 1;
                }
                if valid {
                    rings.push(i);
                    Some(r)
                } else {
                    None
                }
            })
            .collect();
        self.polygons.push(Part {
            reference,
            geometry: Shared::Cleaned(Polygon::new(exterior, holes)),
            rings,
        });
    }

    fn add(
        &mut self,
        g: &Geometry,
        reference: PartRef,
        part: &mut u32,
        feature: Option<usize>,
        path: &mut Vec<usize>,
    ) {
        match g {
            Geometry::MultiPoint(m) => {
                for p in m {
                    self.add_point(p, reference, part);
                }
            }
            Geometry::MultiLineString(m) => {
                for (i, p) in m.into_iter().enumerate() {
                    path.push(i);
                    self.add_line(p, reference, part, feature, path);
                    path.pop();
                }
            }
            Geometry::MultiPolygon(m) => {
                let first = self.polygons.len();
                for (i, p) in m.into_iter().enumerate() {
                    path.push(i);
                    self.add_polygon(p, reference, part, feature, path);
                    path.pop();
                }
                let polygons = &self.polygons[first..];
                let locations = polygons
                    .iter()
                    .map(|p| crate::engine::location::Location::new(&p.geometry))
                    .collect::<Vec<_>>();
                let tree = index::build(
                    polygons
                        .iter()
                        .enumerate()
                        .filter_map(|(i, p)| p.geometry.bounding_rect().map(|r| (i, r))),
                );
                for (i, p) in polygons.iter().enumerate() {
                    for item in tree.locate_in_envelope_intersecting(&index::envelope(
                        p.geometry.bounding_rect().expect("clean polygon"),
                    )) {
                        if item.data > i
                            && crate::engine::validity::multipart(
                                &locations[i],
                                &locations[item.data],
                            )
                        {
                            self.invalid = Some(Invalid::Polygon {
                                fid: reference.fid,
                                part: p.reference.part.unwrap_or(0),
                                ring: None,
                                reason: "invalid multipart: nested shells, overlapping parts or shared edges",
                            });
                        }
                    }
                }
            }
            Geometry::GeometryCollection(m) => {
                for (i, p) in m.into_iter().enumerate() {
                    path.push(i);
                    self.add(p, reference, part, feature, path);
                    path.pop();
                }
            }
            Geometry::Line(l) => self.add_line(
                &LineString(vec![l.start, l.end]),
                reference,
                part,
                None,
                path,
            ),
            Geometry::Rect(r) => self.add_polygon(&r.to_polygon(), reference, part, None, path),
            Geometry::Triangle(t) => self.add_polygon(&t.to_polygon(), reference, part, None, path),
            Geometry::Point(p) => self.add_point(p, reference, part),
            Geometry::LineString(l) => self.add_line(l, reference, part, feature, path),
            Geometry::Polygon(p) => self.add_polygon(p, reference, part, feature, path),
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
        let parts = layer.parts().unwrap();
        assert_eq!(parts.skipped, 3);
        assert_eq!(
            *parts.lines[0].geometry,
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
                reference.at(LayerId(3)),
                FeatureRef {
                    layer: LayerId(3),
                    fid: -12,
                    part: Some(part)
                }
            );
        }
        assert_eq!(layer.features().next().unwrap().1, &geometry);
    }

    /// Lines and polygons of mixed size, some repeated so that their boxes tie.
    fn mixed(count: usize) -> Vec<(i64, Geometry)> {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move |scale: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % scale) as f64 / 4.
        };
        let mut features: Vec<(i64, Geometry)> = Vec::with_capacity(count);
        for fid in 0..count {
            let geometry: Geometry = if fid % 5 == 4 {
                features[fid - 3].1.clone()
            } else {
                let (x, y) = (next(800), next(800));
                let (w, h) = (next(64) + 0.25, next(64) + 0.25);
                let square = |x: f64| Rect::new((x, y), (x + w, y + h)).to_polygon();
                match fid % 3 {
                    0 => line_string![(x: x, y: y), (x: x + w, y: y + h)].into(),
                    1 => square(x).into(),
                    _ => MultiPolygon(vec![square(x), square(x + w + 1.)]).into(),
                }
            };
            features.push((fid as i64, geometry));
        }
        features
    }

    #[test]
    fn line_and_polygon_indexes_search_in_the_order_of_a_bulk_loaded_r_tree() {
        use crate::engine::{
            index::{Index, Search},
            parallel::Workers,
        };
        let features = mixed(3000);
        let built = |boxes: Vec<Option<Rect>>| -> Index {
            index::build(
                boxes
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, r)| r.map(|r| (i, r))),
            )
        };
        let queries = (0..15 * 15)
            .map(|k| {
                let (x, y) = ((k % 15) as f64 * 15., (k / 15) as f64 * 15.);
                index::expanded(Rect::new((x, y), (x + 10., y + 10.)), 0.5)
            })
            .collect::<Vec<_>>();
        for threads in [1, 4] {
            let layer = Layer::from_features("mixed", features.clone());
            let parts = layer.parts().unwrap();
            let lines = built(
                parts
                    .lines
                    .iter()
                    .map(|p| p.geometry.bounding_rect())
                    .collect(),
            );
            let polygons = built(
                parts
                    .polygons
                    .iter()
                    .map(|p| p.geometry.bounding_rect())
                    .collect(),
            );
            let (line_index, polygon_index) =
                Workers::forced(threads, 64, || (parts.line_index(), parts.polygon_index()));
            for (packed, tree) in [(line_index, &lines), (polygon_index, &polygons)] {
                assert!(packed.items().eq(tree.items()), "{threads}");
                for &query in &queries {
                    assert!(packed.search(query).eq(tree.search(query)), "{threads}");
                }
                let pairs = index::pairs(packed, packed, 0., true);
                assert!(pairs.eq(index::pairs(tree, tree, 0., true)), "{threads}");
            }
            let pairs = index::pairs(line_index, polygon_index, 0.5, false);
            assert!(pairs.eq(index::pairs(&lines, &polygons, 0.5, false)));
        }
    }

    #[test]
    fn line_and_polygon_indexes_stop_in_each_step_when_cancelled() {
        use crate::cancel::{self, hooks};
        use crate::engine::{index::Search, parallel::Workers};
        let threads = 4;
        let features = mixed(20_000);
        for lines in [true, false] {
            let step = if lines {
                "index.lines"
            } else {
                "index.polygons"
            };
            let run = |token: &cancel::CancellationToken| {
                let layer = Layer::from_features("mixed", features.clone());
                let parts = layer.parts().unwrap();
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    cancel::scope(Some(token.clone()), || {
                        Workers::forced(threads, 64, || {
                            let index = if lines {
                                parts.line_index()
                            } else {
                                parts.polygon_index()
                            };
                            index.items().count()
                        })
                    })
                }))
            };
            let token = hooks::watched(None);
            assert!(run(&token).expect("index") > 5000);
            let visits = hooks::record(&token).visits;
            // Every checkpoint is passed, so one taken out shows.
            let steps = visits.keys().copied().collect::<Vec<_>>();
            let mut expected = vec!["fill", "index.arrange", "index.bounds", "index.nodes", step];
            expected.sort_unstable();
            assert_eq!(steps, expected);
            for (step, count) in visits {
                for visit in [0, count / 2, count - 1] {
                    let token = hooks::watched(Some((step, visit)));
                    let result = run(&token);
                    let record = hooks::record(&token);
                    assert!(
                        result.expect_err("cancelled").is::<cancel::Cancelled>(),
                        "{step} {visit}"
                    );
                    // Each other thread passes at most one more checkpoint.
                    let after = record.visits[step];
                    assert!(
                        (visit + 1..=visit + threads).contains(&after),
                        "{step} {visit}: {after}"
                    );
                    assert_eq!(record.live, 0);
                }
            }
        }
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
        assert_eq!(layer.parts().err(), Some(Invalid::NonFinite(42)));
    }
}
