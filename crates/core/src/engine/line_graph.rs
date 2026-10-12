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

#[derive(Debug)]
pub(crate) struct GraphPart {
    pub reference: FeatureRef,
    pub nodes: Vec<usize>,
    pub coords: Vec<Coord>,
    pub is_closed: bool,
    original: Shared<LineString>,
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
    pub edge: usize,
    pub part: usize,
}
#[derive(Debug)]
pub(crate) struct LineGraph {
    pub coords: Vec<Coord>,
    pub parts: Vec<GraphPart>,
    pub edges: Vec<Edge>,
    pub incident: Incidences,
    pub uses: Vec<EdgeUse>,
    sources: Vec<Segment>,
    by_feature: Cache<BTreeMap<(LayerId, i64), FeatureGraph>>,
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
        let retain_coords = layers
            .iter()
            .any(|(_, mode)| *mode == InputMode::PolygonBoundary);
        let mut parts = Vec::new();
        let mut segments = Vec::new();
        let mut endpoints = Vec::new();
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
                    coords: if retain_coords {
                        Vec::with_capacity(geometry.0.len())
                    } else {
                        Vec::new()
                    },
                    is_closed: closed,
                    original: geometry.clone(),
                });
                if !closed {
                    endpoints.extend([geometry.0[0], geometry.0[geometry.0.len() - 1]]);
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
                            add_cut(&mut split, i, line, point);
                            add_cut(&mut split, j, other, point);
                        }
                        LineIntersection::Collinear { intersection } => {
                            for p in [intersection.start, intersection.end] {
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
            for &point in &endpoints {
                let mut candidates = tree
                    .locate_in_envelope_intersecting(&index::at(point, t.tolerance()))
                    .map(|i| i.data)
                    .collect::<Vec<_>>();
                candidates.sort_unstable();
                for i in candidates {
                    let line = segments[i].line(&parts);
                    let (_, projected) = projection(line, point);
                    if distance(point, projected) <= t.tolerance() {
                        add_cut(&mut split, i, line, projected);
                        cracked.push(projected);
                    }
                }
            }
        }
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
            let mut coords = Vec::new();
            for point in cuts {
                let node = if let Some(indexed) = &indexed {
                    indexed.node(point)
                } else {
                    cluster.as_mut().expect("tolerance cluster").insert(point)
                };
                if nodes.last() != Some(&node) {
                    nodes.push(node);
                    coords.push(point);
                }
            }
            let part = &mut parts[s.part];
            for (&node, &point) in nodes.iter().zip(&coords) {
                if part.nodes.last() != Some(&node) {
                    part.nodes.push(node);
                    if retain_coords {
                        part.coords.push(point);
                    }
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
        let mut values = vec![Incident { edge: 0, part: 0 }; uses.len() * 2];
        for (id, edge) in edges.iter().enumerate() {
            for usage in &uses[edge.uses.clone()] {
                for node in edge.nodes {
                    values[cursor[node]] = Incident {
                        edge: id,
                        part: usage.part,
                    };
                    cursor[node] += 1;
                }
            }
        }
        let incident = Incidences { offsets, values };
        Ok(Self {
            by_feature: Cache::new(),
            coords,
            sources: segments,
            uses,
            parts,
            edges,
            incident,
        })
    }
}
