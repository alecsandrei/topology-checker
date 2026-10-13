use crate::cache::Cache;
use crate::{
    Error, FeatureRef, LayerId, Topology,
    engine::{
        cluster::{Cluster, distance},
        index,
        storage::Shared,
    },
};
use geo::{
    BoundingRect, Coord, Line, LineString,
    line_intersection::{LineIntersection, line_intersection},
};
use num_rational::BigRational;
use num_traits::ToPrimitive;
use std::collections::{BTreeMap, HashMap};
type FeatureKey = (LayerId, i64);
#[cfg(test)]
type MatchPairs = BTreeMap<(FeatureKey, FeatureKey), Vec<usize>>;

#[cfg(test)]
thread_local! {
    pub(crate) static MATCHING_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug)]
pub(crate) struct GraphPart {
    pub reference: FeatureRef,
    pub nodes: Vec<usize>,
    pub is_closed: bool,
    original: Shared<LineString>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Layer, Rule};

    #[test]
    fn source_junction_incidence_is_stable_under_translation() {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/geometry_cases.json")).unwrap();
        let shapes = data["dc_junction_contacts"]["geometries"]
            .as_array()
            .unwrap();
        for start in [0, 2] {
            for shift in [false, true] {
                let mut t = Topology::new(0.001).unwrap();
                let layers = [start, start + 1].map(|index| {
                    let coords = shapes[index]["coordinates"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| {
                            let x = p[0].as_f64().unwrap() + if shift { 1048576. } else { 0. };
                            let y = p[1].as_f64().unwrap() - if shift { 524288. } else { 0. };
                            (x, y)
                        })
                        .collect::<Vec<_>>();
                    t.add_layer(Layer::from_geometries(
                        "source",
                        [LineString::from(coords).into()],
                    ))
                });
                let g = t.graph(layers.to_vec()).unwrap();
                assert!(g.matched_pair((layers[0], 0), (layers[1], 0)).is_empty());
                t.add_rule(
                    Rule::LineMustNotIntersectOrTouchInteriorWith,
                    layers[0],
                    Some(layers[1]),
                )
                .unwrap();
                let report = t.validate().unwrap();
                assert_eq!(report.violation_count(), 1);
                assert_eq!(
                    report.rules[0].violations[0].kind,
                    crate::ViolationKind::TouchInterior
                );
                let mut streamed = Vec::new();
                t.validate_into(|_, v| {
                    streamed.push(v);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(streamed, report.rules[0].violations);
            }
        }
    }

    #[test]
    fn feature_streaming_visits_sparse_matches_and_canonical_directions_once() {
        for tolerance in [0., 0.001] {
            for rule in [
                Rule::LineMustNotOverlap,
                Rule::LineMustNotIntersect,
                Rule::LineMustNotIntersectOrTouchInterior,
                Rule::LineMustNotOverlapWith,
                Rule::LineMustNotIntersectWith,
                Rule::LineMustNotIntersectOrTouchInteriorWith,
            ] {
                let mut t = Topology::new(tolerance).unwrap();
                let lines = (0..512)
                    .map(|i| {
                        let x = (i / 2) as f64 * 20.;
                        let shift = (i % 2) as f64;
                        (
                            i as i64 * 7 - 91,
                            LineString::from(vec![(x + shift, 0.), (x + shift + 10., 0.)]).into(),
                        )
                    })
                    .collect::<Vec<_>>();
                let a = t.add_layer(Layer::from_features("primary", lines.clone()));
                let other = rule
                    .inputs()
                    .1
                    .map(|_| t.add_layer(Layer::from_features("target", lines)));
                t.add_rule(rule, a, other).unwrap();
                MATCHING_WORK.with(|n| n.set(0));
                let mut streamed = Vec::new();
                t.validate_into(|_, v| {
                    streamed.push(v);
                    Ok::<_, ()>(())
                })
                .unwrap();
                let work = MATCHING_WORK.with(std::cell::Cell::get);
                assert!(
                    work < 16_384,
                    "{rule}: {work} visits must be linear in sparse intervals"
                );
                assert!(!streamed.is_empty());
                assert_eq!(
                    streamed,
                    t.validate().unwrap().rules[0].violations,
                    "{rule}"
                );
            }
        }
    }
}
impl GraphPart {
    pub fn closed(&self) -> bool {
        self.is_closed
    }
    pub fn endpoint(&self, node: usize) -> bool {
        !self.closed() && (self.nodes.first() == Some(&node) || self.nodes.last() == Some(&node))
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct EdgeUse {
    pub part: usize,
    pub source: usize,
}
#[derive(Debug)]
pub(crate) struct Edge {
    pub nodes: [usize; 2],
    pub uses: std::ops::Range<usize>,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Incident {
    pub part: usize,
}
#[derive(Debug)]
pub(crate) struct LineGraph {
    pub coords: Vec<Coord>,
    pub parts: Vec<GraphPart>,
    pub edges: Vec<Edge>,
    pub incident: Incidences,
    pub contacts: BTreeMap<usize, Vec<Contact>>,
    pub uses: Vec<EdgeUse>,
    sources: Vec<Segment>,
    matches: Cache<Vec<Matched>>,
    #[cfg(test)]
    match_pairs: Cache<MatchPairs>,
    match_features: Cache<BTreeMap<FeatureKey, Vec<usize>>>,
    canonical_directions: Cache<Vec<bool>>,
    pub tolerance: f64,
    by_feature: Cache<BTreeMap<(LayerId, i64), FeatureGraph>>,
}
#[derive(Debug)]
pub(crate) struct Matched {
    pub sources: [usize; 2],
    pub spans: [[f64; 2]; 2],
}
/// Original incidences at one actual source contact, before node clustering.
#[derive(Debug)]
pub(crate) struct Contact {
    pub parts: [usize; 2],
    pub interior: [bool; 2],
    pub points: [Coord; 2],
    pub sources: [usize; 2],
}
#[derive(Debug, Default)]
pub(crate) struct FeatureGraph {
    pub parts: Vec<usize>,
    pub edges: Vec<usize>,
    pub nodes: Vec<usize>,
}
/// Compact contiguous incidences; construction allocation is released on caching.
#[derive(Debug)]
pub(crate) struct Incidences {
    offsets: Vec<usize>,
    values: Vec<Incident>,
}
impl Incidences {
    pub fn iter(&self) -> impl Iterator<Item = &[Incident]> {
        self.offsets.windows(2).map(|o| &self.values[o[0]..o[1]])
    }
}
impl std::ops::Index<usize> for Incidences {
    type Output = [Incident];
    fn index(&self, i: usize) -> &Self::Output {
        &self.values[self.offsets[i]..self.offsets[i + 1]]
    }
}
impl Edge {
    pub fn uses<'a>(&self, graph: &'a LineGraph) -> &'a [EdgeUse] {
        &graph.uses[self.uses.clone()]
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Segment {
    pub part: usize,
    pub start: usize,
}
impl Segment {
    fn line(self, parts: &[GraphPart]) -> Line {
        let coords = &parts[self.part].original.0;
        Line::new(coords[self.start], coords[self.start + 1])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum InputMode {
    Linework,
    PolygonBoundary,
}

/// Project onto a segment without squaring its unscaled length.
pub(crate) fn projection(line: Line, point: Coord) -> (f64, Coord) {
    let dx = line.end.x - line.start.x;
    let dy = line.end.y - line.start.y;
    let scale = dx.abs().max(dy.abs());
    if scale == 0. {
        return (0., line.start);
    }
    let x = dx / scale;
    let y = dy / scale;
    let parameter = (((point.x - line.start.x) / scale * x + (point.y - line.start.y) / scale * y)
        / (x * x + y * y))
        .clamp(0., 1.);
    (
        parameter,
        if parameter == 0. {
            line.start
        } else if parameter == 1. {
            line.end
        } else {
            Coord {
                x: line.start.x + parameter * dx,
                y: line.start.y + parameter * dy,
            }
        },
    )
}
fn parameter(line: Line, point: Coord) -> f64 {
    let dx = line.end.x - line.start.x;
    let dy = line.end.y - line.start.y;
    if dx.abs() >= dy.abs() {
        (point.x - line.start.x) / dx
    } else {
        (point.y - line.start.y) / dy
    }
}

/// Round the exact intersection of the input binary floats once. Pairwise floating point
/// formulas can otherwise round a concurrent crossing differently for each segment pair,
/// creating spurious tiny edges even at zero user tolerance. Endpoint contacts retain their
/// original coordinates and do not pay for rational arithmetic.
pub(crate) fn proper_intersection(a: Line, b: Line) -> Option<Coord> {
    let [ax, ay, bx, by, cx, cy, dx, dy] = [
        a.start.x, a.start.y, a.end.x, a.end.y, b.start.x, b.start.y, b.end.x, b.end.y,
    ]
    .map(BigRational::from_float);
    let (ax, ay, bx, by, cx, cy, dx, dy) = (ax?, ay?, bx?, by?, cx?, cy?, dx?, dy?);
    let rx = bx - &ax;
    let ry = by - &ay;
    let sx = dx - &cx;
    let sy = dy - &cy;
    let determinant = &rx * &sy - &ry * &sx;
    if determinant == BigRational::from_integer(0.into()) {
        return None;
    }
    let t = ((cx - &ax) * sy - (cy - &ay) * sx) / determinant;
    Some(Coord {
        x: (ax + &t * rx).to_f64()?,
        y: (ay + t * ry).to_f64()?,
    })
}
pub(crate) fn collapsed(line: &LineString, tolerance: f64) -> bool {
    if tolerance == 0. {
        return false;
    }
    let Some(bounds) = line.bounding_rect() else {
        return true;
    };
    if bounds.width() > tolerance || bounds.height() > tolerance {
        return false;
    }
    line.0
        .iter()
        .enumerate()
        .all(|(i, &p)| line.0[i + 1..].iter().all(|&q| distance(p, q) <= tolerance))
}
fn add_cut(cuts: &mut HashMap<usize, Vec<Coord>>, source: usize, line: Line, point: Coord) {
    if point != line.start && point != line.end {
        cuts.entry(source).or_default().push(point);
    }
}
fn coordinate_order(a: &Coord, b: &Coord) -> std::cmp::Ordering {
    a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y))
}
// Zero tolerance has no grouping after contact discovery. Keep a sorted index
// into the existing coordinate array instead of retaining the coordinate hash
// while edge storage is allocated. Node identities keep their original order.
#[derive(Debug)]
struct IndexedNodes {
    coords: Vec<Coord>,
    order: Vec<usize>,
}
impl IndexedNodes {
    fn compare(a: Coord, b: Coord) -> std::cmp::Ordering {
        a.x.partial_cmp(&b.x)
            .expect("finite coordinate")
            .then_with(|| a.y.partial_cmp(&b.y).expect("finite coordinate"))
    }
    fn new(coords: Vec<Coord>) -> Self {
        let mut order = (0..coords.len()).collect::<Vec<_>>();
        order.sort_unstable_by(|&a, &b| Self::compare(coords[a], coords[b]));
        Self { coords, order }
    }
    fn node(&self, p: Coord) -> usize {
        self.order[self
            .order
            .binary_search_by(|&id| Self::compare(self.coords[id], p))
            .expect("all contacts discovered before edge construction")]
    }
}
impl LineGraph {
    pub fn canonical_reversed(&self, part: usize) -> bool {
        self.canonical_directions.get_or_init(|| {
            self.parts
                .iter()
                .map(|p| {
                    crate::cancel::check();
                    #[cfg(test)]
                    MATCHING_WORK.with(|n| n.set(n.get() + p.original.0.len()));
                    p.original
                        .0
                        .iter()
                        .zip(p.original.0.iter().rev())
                        .map(|(a, b)| coordinate_order(a, b))
                        .find(|o| !o.is_eq())
                        .is_some_and(|o| o.is_gt())
                })
                .collect()
        })[part]
    }
    /// Streaming keeps only the selected feature's intervals in its working set.
    pub fn selected_matches(
        &self,
        feature: Option<FeatureKey>,
    ) -> Box<dyn Iterator<Item = &Matched> + '_> {
        let Some(feature) = feature else {
            return Box::new(self.matches().iter());
        };
        let ids = self.match_features.get_or_init(|| {
            let mut features = BTreeMap::<_, Vec<_>>::new();
            for (id, m) in self.matches().iter().enumerate() {
                crate::cancel::check();
                let [a, b] = m
                    .sources
                    .map(|s| self.parts[self.segment(s).part].reference.feature());
                features.entry(a).or_default().push(id);
                if a != b {
                    features.entry(b).or_default().push(id);
                }
            }
            features
        });
        Box::new(
            ids.get(&feature)
                .into_iter()
                .flatten()
                .map(|&id| &self.matches()[id]),
        )
    }
    #[cfg(test)]
    pub fn matched_pair(&self, a: (LayerId, i64), b: (LayerId, i64)) -> &[usize] {
        let key = if a < b { (a, b) } else { (b, a) };
        self.match_pairs
            .get_or_init(|| {
                let mut pairs = BTreeMap::<_, Vec<_>>::new();
                for (i, m) in self.matches().iter().enumerate() {
                    let [a, b] = m
                        .sources
                        .map(|s| self.parts[self.segment(s).part].reference.feature());
                    let key = if a < b { (a, b) } else { (b, a) };
                    pairs.entry(key).or_default().push(i);
                }
                pairs
            })
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    pub fn original(&self, part: usize) -> &LineString {
        &self.parts[part].original
    }
    pub fn segment(&self, source: usize) -> Segment {
        self.sources[source]
    }
    pub fn matches(&self) -> &[Matched] {
        self.matches.get_or_init(|| {
            let tree = index::build(
                self.sources
                    .iter()
                    .enumerate()
                    .map(|(i, _)| (i, self.source(i).bounding_rect())),
            );
            let mut result = Vec::new();
            for (a, b) in index::pairs(&tree, &tree, self.tolerance, true) {
                crate::cancel::check();
                if let Some(spans) =
                    super::matching::interval(self.source(a), self.source(b), self.tolerance)
                {
                    result.push(Matched {
                        sources: [a, b],
                        spans,
                    });
                }
            }
            result.sort_unstable_by_key(|m| m.sources);
            result
        })
    }
    pub fn source(&self, index: usize) -> Line {
        self.sources[index].line(&self.parts)
    }
    pub fn feature(&self, key: (LayerId, i64)) -> Option<&FeatureGraph> {
        self.by_feature
            .get_or_init(|| {
                let mut features: BTreeMap<_, FeatureGraph> = BTreeMap::new();
                for (id, edge) in self.edges.iter().enumerate() {
                    for usage in edge.uses(self) {
                        let entry = features
                            .entry(self.parts[usage.part].reference.feature())
                            .or_default();
                        if entry.edges.last() != Some(&id) {
                            entry.edges.push(id);
                        }
                    }
                }
                for (i, part) in self.parts.iter().enumerate() {
                    features
                        .entry(part.reference.feature())
                        .or_default()
                        .parts
                        .push(i);
                    features
                        .entry(part.reference.feature())
                        .or_default()
                        .nodes
                        .extend(&part.nodes);
                }
                for feature in features.values_mut() {
                    feature.nodes.sort_unstable();
                    feature.nodes.dedup();
                }
                features
            })
            .get(&key)
    }
    pub fn build(t: &Topology, layers: &[(LayerId, InputMode)]) -> Result<Self, Error> {
        let mut parts = Vec::new();
        let mut segments = Vec::new();
        let mut endpoints = Vec::new();
        let mut endpoint_sources = Vec::new();
        let mut coordinates = Vec::new();
        for &(layer, mode) in layers {
            let input = t.parts(layer)?;
            let lines: Box<dyn Iterator<Item = (crate::model::PartRef, &Shared<LineString>)> + '_> =
                match mode {
                    InputMode::Linework => {
                        Box::new(input.lines.iter().map(|p| (p.reference, &p.geometry)))
                    }
                    InputMode::PolygonBoundary => Box::new(
                        crate::engine::cross::boundaries(input)
                            .iter()
                            .map(|p| (p.reference, &p.geometry)),
                    ),
                };
            for (reference, geometry) in lines {
                if mode == InputMode::Linework && collapsed(geometry, t.tolerance()) {
                    continue;
                }
                coordinates.extend_from_slice(&geometry.0);
                let closed = geometry.0.first() == geometry.0.last();
                let id = parts.len();
                parts.push(GraphPart {
                    // Graphs belong to one topology, so parts carry final layer IDs.
                    reference: reference.at(layer),
                    nodes: Vec::with_capacity(geometry.0.len()),
                    is_closed: closed,
                    original: geometry.clone(),
                });
                if !closed {
                    endpoints.extend([geometry.0[0], geometry.0[geometry.0.len() - 1]]);
                    endpoint_sources
                        .extend([segments.len(), segments.len() + geometry.0.len() - 2]);
                }
                segments.extend((0..geometry.0.len() - 1).map(|start| Segment { start, part: id }));
            }
        }
        let tree = index::build(
            segments
                .iter()
                .enumerate()
                .map(|(i, s)| (i, s.line(&parts).bounding_rect())),
        );
        // Most source segments have no interior contact. Store only actual cuts,
        // avoiding one empty Vec allocation header per original segment.
        let mut split = HashMap::new();
        // Multipart endpoint parity belongs to original feature coordinates.
        // Nearby clustered endpoints do not form an original through passage.
        let mut endpoint_counts = HashMap::<_, usize>::new();
        for (&point, &source) in endpoints.iter().zip(&endpoint_sources) {
            let feature = parts[segments[source].part].reference.feature();
            *endpoint_counts
                .entry((
                    feature,
                    super::cluster::bits(point.x),
                    super::cluster::bits(point.y),
                ))
                .or_default() += 1;
        }
        let mut original_contacts = Vec::new();
        let mut contact = |a: usize, b: usize, points: [Coord; 2], coordinate: Coord| {
            let sources = [a, b];
            let [a, b] = sources.map(|source| segments[source].part);
            if parts[a].reference.feature() == parts[b].reference.feature() {
                return;
            }
            let interior = [0, 1].map(|side| {
                let p = &parts[[a, b][side]];
                if p.closed() {
                    return true;
                }
                let feature = p.reference.feature();
                let endpoints = [p.original.0[0], *p.original.0.last().expect("source")];
                let endpoint = endpoints
                    .into_iter()
                    .filter(|&point| distance(points[side], point) <= t.tolerance())
                    .min_by(|&a, &b| {
                        distance(points[side], a).total_cmp(&distance(points[side], b))
                    });
                endpoint.is_none_or(|point| {
                    endpoint_counts[&(
                        feature,
                        super::cluster::bits(point.x),
                        super::cluster::bits(point.y),
                    )]
                        .is_multiple_of(2)
                })
            });
            // Endpoint junctions need no retained meeting evidence.
            if interior[0] || interior[1] {
                original_contacts.push((
                    coordinate,
                    Contact {
                        parts: [a, b],
                        interior,
                        points,
                        sources,
                    },
                ));
            }
        };
        for (i, s) in segments.iter().enumerate() {
            crate::cancel::check();
            let line = s.line(&parts);
            for item in tree.locate_in_envelope_intersecting(&index::envelope(line.bounding_rect()))
            {
                let j = item.data;
                if j <= i {
                    continue;
                }
                let other = segments[j].line(&parts);
                if let Some(g) = line_intersection(line, other) {
                    match g {
                        LineIntersection::SinglePoint {
                            intersection,
                            is_proper,
                        } => {
                            let point = if is_proper {
                                proper_intersection(line, other).unwrap_or(intersection)
                            } else {
                                intersection
                            };
                            contact(i, j, [point, point], point);
                            add_cut(&mut split, i, line, point);
                            add_cut(&mut split, j, other, point);
                        }
                        LineIntersection::Collinear { intersection } => {
                            for p in [intersection.start, intersection.end] {
                                contact(i, j, [p, p], p);
                                add_cut(&mut split, i, line, p);
                                add_cut(&mut split, j, other, p);
                            }
                        }
                    }
                }
            }
        }
        let mut cracked = Vec::new();
        // Endpoint cracking has no effect on the immutable input geometries.
        if t.tolerance() > 0. {
            for (&point, &source) in endpoints.iter().zip(&endpoint_sources) {
                let mut candidates = tree
                    .locate_in_envelope_intersecting(&index::at(point, t.tolerance()))
                    .map(|i| i.data)
                    .collect::<Vec<_>>();
                candidates.sort_unstable();
                for i in candidates {
                    let line = segments[i].line(&parts);
                    let (_, projected) = projection(line, point);
                    if distance(point, projected) <= t.tolerance() {
                        contact(source, i, [point, projected], point);
                        add_cut(&mut split, i, line, projected);
                        cracked.push(projected);
                    }
                }
            }
        }
        // Endpoint parity is construction evidence, not retained graph storage.
        // Release it before allocating noded edges and incidences.
        drop(endpoint_counts);
        drop(endpoint_sources);
        drop(tree);
        let mut cluster = Cluster::new(t.tolerance());
        endpoints.sort_by(coordinate_order);
        endpoints.dedup();
        for point in endpoints {
            cluster.insert(point);
        }
        cracked.sort_by(coordinate_order);
        cracked.dedup();
        for point in cracked {
            cluster.insert(point);
        }
        // All contact coordinates are discovered against source segments first.
        // Canonical insertion makes clustering independent of FIDs and path direction.
        let mut contacts = coordinates;
        contacts.extend(split.values().flatten().copied());
        contacts.sort_by(coordinate_order);
        contacts.dedup();
        for point in contacts {
            cluster.insert(point);
        }
        let mut cluster = Some(cluster);
        let indexed = (t.tolerance() == 0.)
            .then(|| IndexedNodes::new(cluster.take().expect("cluster").finish()));
        let mut raw_edges: Vec<([usize; 2], EdgeUse)> = Vec::with_capacity(segments.len());
        for (source, s) in segments.iter().enumerate() {
            crate::cancel::check();
            let line = s.line(&parts);
            let mut cuts = split.remove(&source).unwrap_or_default();
            cuts.extend([line.start, line.end]);
            cuts.sort_by(|a, b| parameter(line, *a).total_cmp(&parameter(line, *b)));
            cuts.dedup();
            let mut nodes = Vec::new();
            for point in cuts {
                let node = if let Some(indexed) = &indexed {
                    indexed.node(point)
                } else {
                    cluster.as_mut().expect("tolerance cluster").insert(point)
                };
                if nodes.last() != Some(&node) {
                    nodes.push(node);
                }
            }
            let part = &mut parts[s.part];
            for &node in &nodes {
                if part.nodes.last() != Some(&node) {
                    part.nodes.push(node);
                }
            }
            for pair in nodes.windows(2) {
                let [a, b] = [pair[0], pair[1]];
                if a == b {
                    continue;
                }
                raw_edges.push((
                    [a.min(b), a.max(b)],
                    EdgeUse {
                        part: s.part,
                        source,
                    },
                ));
            }
        }
        let mut contacts = BTreeMap::<_, Vec<_>>::new();
        for (coordinate, contact) in original_contacts {
            crate::cancel::check();
            let node = if let Some(indexed) = &indexed {
                indexed.node(coordinate)
            } else {
                cluster
                    .as_mut()
                    .expect("tolerance cluster")
                    .insert(coordinate)
            };
            contacts.entry(node).or_default().push(contact);
        }
        let coords = if let Some(indexed) = indexed {
            indexed.coords
        } else {
            cluster.expect("tolerance cluster").finish()
        };
        raw_edges.sort_unstable_by_key(|e| (e.0, e.1.part, e.1.source));
        let mut uses = Vec::with_capacity(raw_edges.len());
        let mut groups = 0;
        for i in 0..raw_edges.len() {
            crate::cancel::check();
            let (nodes, usage) = raw_edges[i];
            let start = uses.len();
            uses.push(usage);
            // Compact group ranges in the original entries. The final collect
            // can reuse this allocation because source and target have the same
            // size and alignment; no second full edge array is needed.
            if groups == 0 || raw_edges[groups - 1].0 != nodes {
                raw_edges[groups] = (
                    nodes,
                    EdgeUse {
                        part: start,
                        source: start + 1,
                    },
                );
                groups += 1;
            } else {
                raw_edges[groups - 1].1.source = start + 1;
            }
        }
        raw_edges.truncate(groups);
        let edges = raw_edges
            .into_iter()
            .map(|(nodes, range)| Edge {
                nodes,
                uses: range.part..range.source,
            })
            .collect::<Vec<_>>();
        let mut offsets = vec![0; coords.len() + 1];
        for edge in &edges {
            for node in edge.nodes {
                offsets[node + 1] += edge.uses.len();
            }
        }
        for i in 1..offsets.len() {
            offsets[i] += offsets[i - 1];
        }
        let mut cursor = offsets[..coords.len()].to_vec();
        let mut values = vec![Incident { part: 0 }; uses.len() * 2];
        for edge in &edges {
            for usage in &uses[edge.uses.clone()] {
                for node in edge.nodes {
                    values[cursor[node]] = Incident { part: usage.part };
                    cursor[node] += 1;
                }
            }
        }
        let incident = Incidences { offsets, values };
        Ok(Self {
            matches: Cache::new(),
            #[cfg(test)]
            match_pairs: Cache::new(),
            match_features: Cache::new(),
            canonical_directions: Cache::new(),
            tolerance: t.tolerance(),
            by_feature: Cache::new(),
            coords,
            sources: segments,
            uses,
            parts,
            edges,
            incident,
            contacts,
        })
    }
}
