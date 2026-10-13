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
    static PASSAGE_ROLE_SLOTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static JUNCTION_SUPPORT_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
    fn multipart_restoration_allocates_roles_only_for_self_participants() {
        for self_group in [false, true] {
            let mut t = Topology::new(1.).unwrap();
            let a = LineString::from(vec![(0., 0.), (3., 0.)]);
            let b = geo::MultiLineString(
                (0..6)
                    .map(|i| {
                        LineString::from(vec![
                            (i as f64 / 2., 0.25),
                            ((i + 1) as f64 / 2., 0.25),
                            ((i + 1) as f64 / 2., -10.),
                        ])
                    })
                    .collect(),
            );
            let mut features = (0..512)
                .map(|i| {
                    (
                        i,
                        LineString::from(vec![
                            (100. + i as f64 * 20., 0.),
                            (110. + i as f64 * 20., 0.),
                        ])
                        .into(),
                    )
                })
                .collect::<Vec<(i64, geo::Geometry)>>();
            if self_group {
                let mut parts = b.0;
                parts.push(a);
                features.push((-1, geo::MultiLineString(parts).into()));
            } else {
                features.extend([(-1, a.into()), (-2, b.into())]);
            }
            let layer = t.add_layer(Layer::from_features("support", features));
            let g = t.graph(vec![layer]).unwrap();
            PASSAGE_ROLE_SLOTS.with(|n| n.set(0));
            assert!(g.selected_matches(None).next().is_some());
            assert_eq!(
                PASSAGE_ROLE_SLOTS.with(|n| n.get()),
                if self_group { 7 } else { 0 }
            );
        }
        // Independent short pairs enter restoration but cannot gain multipart support.
        let mut t = Topology::new(1.).unwrap();
        let shapes = (0..512).flat_map(|i| {
            let x = i as f64 * 20.;
            [
                LineString::from(vec![(x, 0.), (x + 0.5, 0.), (x + 0.5, 10.)]),
                LineString::from(vec![(x, 0.25), (x + 0.5, 0.25), (x + 0.5, -10.)]),
            ]
            .map(geo::Geometry::from)
        });
        let layer = t.add_layer(Layer::from_geometries("sparse", shapes));
        let g = t.graph(vec![layer]).unwrap();
        PASSAGE_ROLE_SLOTS.with(|n| n.set(0));
        assert!(g.selected_matches(None).next().is_none());
        assert_eq!(PASSAGE_ROLE_SLOTS.with(|n| n.get()), 0);
    }

    #[test]
    fn sustained_support_reuses_source_traversals_and_local_candidates() {
        let vertices = 2048;
        let mut t = Topology::new(1.).unwrap();
        let layer = t.add_layer(Layer::from_geometries(
            "nearby",
            [0., 0.25].map(|y| {
                LineString(
                    (0..vertices)
                        .map(|i| Coord {
                            x: i as f64 / 2.,
                            y,
                        })
                        .collect(),
                )
                .into()
            }),
        ));
        let g = t.graph(vec![layer]).unwrap();
        JUNCTION_SUPPORT_WORK.with(|n| n.set(0));
        assert!(g.selected_matches(None).next().is_some());
        assert!(JUNCTION_SUPPORT_WORK.with(|n| n.get()) < 100 * vertices);
    }

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
    /// Raw intervals still contribute to coverage before its final length test.
    pub overlap: bool,
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
    let (_, [x, y]) = exact_crossing(a, b)?;
    Some(Coord {
        x: x.to_f64()?,
        y: y.to_f64()?,
    })
}
/// The exact crossing of the lines through `a` and `b`, from their binary
/// floats: its parameter along `a` and its rational coordinates. `None` for
/// parallel or non-finite input.
pub(crate) fn exact_crossing(a: Line, b: Line) -> Option<(BigRational, [BigRational; 2])> {
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
    let point = [ax + &t * rx, ay + &t * ry];
    Some((t, point))
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
            return Box::new(self.matches().iter().filter(|m| m.overlap));
        };
        let ids = self.match_features.get_or_init(|| {
            let mut features = BTreeMap::<_, Vec<_>>::new();
            for (id, m) in self.matches().iter().enumerate() {
                crate::cancel::check();
                if !m.overlap {
                    continue;
                }
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
                    if !m.overlap {
                        continue;
                    }
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
            let mut evidence = Vec::new();
            for (a, b) in index::pairs(&tree, &tree, self.tolerance, true) {
                crate::cancel::check();
                if let Some(support) =
                    super::matching::evidence(self.source(a), self.source(b), self.tolerance)
                {
                    result.push(Matched {
                        sources: [a, b],
                        spans: support.spans,
                        overlap: true,
                    });
                    if self.tolerance > 0. {
                        evidence.push(support);
                    }
                }
            }
            if self.tolerance > 0. && !result.is_empty() {
                self.retain_sustained_matches(&tree, &mut result, &evidence);
            }
            result.sort_unstable_by_key(|m| m.sources);
            result
        })
    }

    /// Filter connected original support, before output endpoint caps are used.
    /// A point junction can cover a short passage, but cannot establish overlap.
    fn retain_sustained_matches(
        &self,
        tree: &index::Index,
        matches: &mut [Matched],
        evidence: &[super::matching::Evidence],
    ) {
        let mut groups = BTreeMap::<_, Vec<_>>::new();
        for (i, m) in matches.iter().enumerate() {
            crate::cancel::check();
            #[cfg(test)]
            JUNCTION_SUPPORT_WORK.with(|n| n.set(n.get() + 1));
            let parts = m.sources.map(|s| self.segment(s).part);
            let short = [0, 1].iter().any(|&side| {
                let line = self.source(m.sources[side]);
                (evidence[i].support[side][1] - evidence[i].support[side][0])
                    * distance(line.start, line.end)
                    <= self.tolerance
            });
            if parts[0] != parts[1] && (!evidence[i].exact || short) {
                groups.entry(parts).or_default();
            }
        }
        if groups.is_empty() {
            return;
        }
        for (i, m) in matches.iter().enumerate() {
            crate::cancel::check();
            let parts = m.sources.map(|s| self.segment(s).part);
            if let Some(ids) = groups.get_mut(&parts) {
                ids.push(i);
            }
        }
        // Original traversal distances distinguish a compact bent run from a
        // short junction approach. This temporary index is built once per part.
        let mut positions = vec![Vec::new(); self.parts.len()];
        let mut uncertainty = vec![0.; self.parts.len()];
        for parts in groups.keys() {
            for &part in parts {
                if !positions[part].is_empty() {
                    continue;
                }
                let mut total = 0.;
                let mut bounds = super::certified::Interval(0., 0.);
                positions[part].push(total);
                for line in self.original(part).lines() {
                    crate::cancel::check();
                    #[cfg(test)]
                    JUNCTION_SUPPORT_WORK.with(|n| n.set(n.get() + 1));
                    total += distance(line.start, line.end);
                    use super::certified::Interval;
                    let x =
                        Interval(line.end.x, line.end.x).sub(Interval(line.start.x, line.start.x));
                    let y =
                        Interval(line.end.y, line.end.y).sub(Interval(line.start.y, line.start.y));
                    bounds = bounds.add(x.square().add(y.square()).sqrt());
                    uncertainty[part] = bounds.1 - bounds.0;
                    if !total.is_finite()
                        || !uncertainty[part].is_finite()
                        || uncertainty[part] >= self.tolerance
                    {
                        uncertainty[part] = f64::INFINITY;
                        break;
                    }
                    positions[part].push(total);
                }
            }
        }
        // Uncertifiable cumulative distances cannot reject original evidence.
        // This includes overflow and short intervals after very large prefixes.
        groups.retain(|parts, _| parts.iter().all(|&part| uncertainty[part].is_finite()));
        if groups.is_empty() {
            return;
        }
        let along = |source: usize, parameter: f64| {
            let segment = self.segment(source);
            let p = &positions[segment.part];
            p[segment.start] + parameter * (p[segment.start + 1] - p[segment.start])
        };
        let mut junctions = BTreeMap::<_, Vec<[f64; 2]>>::new();
        for (a, b) in index::pairs(tree, tree, self.tolerance, true) {
            crate::cancel::check();
            #[cfg(test)]
            JUNCTION_SUPPORT_WORK.with(|n| n.set(n.get() + 1));
            let parts = [self.segment(a).part, self.segment(b).part];
            if !groups.contains_key(&parts) {
                continue;
            }
            let lines = [self.source(a), self.source(b)];
            if let Some(LineIntersection::SinglePoint {
                intersection,
                is_proper: false,
            }) = line_intersection(lines[0], lines[1])
            {
                junctions.entry(parts).or_default().push([
                    along(a, projection(lines[0], intersection).0),
                    along(b, projection(lines[1], intersection).0),
                ]);
            }
            for side in 0..2 {
                let part = &self.parts[parts[side]];
                if part.is_closed {
                    continue;
                }
                let source = self.segment([a, b][side]);
                for (position, p) in [
                    (source.start, lines[side].start),
                    (source.start + 1, lines[side].end),
                ] {
                    if position != 0 && position != part.original.0.len() - 1 {
                        continue;
                    }
                    let (parameter, projected) = projection(lines[1 - side], p);
                    if distance(p, projected) <= self.tolerance {
                        let mut contact = [0.; 2];
                        contact[side] = positions[parts[side]][position];
                        contact[1 - side] = along([a, b][1 - side], parameter);
                        junctions.entry(parts).or_default().push(contact);
                    }
                }
            }
        }
        let mut rejected = vec![false; matches.len()];
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for (parts, ids) in groups {
            crate::cancel::check();
            let mut parent = (0..ids.len()).collect::<Vec<_>>();
            for (side, &part) in parts.iter().enumerate() {
                let mut spans = ids
                    .iter()
                    .enumerate()
                    .map(|(local, &id)| {
                        crate::cancel::check();
                        let source = matches[id].sources[side];
                        let start = self.segment(source).start as f64;
                        let span = evidence[id].support[side];
                        (start + span[0], start + span[1], local)
                    })
                    .collect::<Vec<_>>();
                spans.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                if self.parts[part].is_closed {
                    let first = spans.first().expect("support");
                    let last = spans
                        .iter()
                        .max_by(|a, b| a.1.total_cmp(&b.1))
                        .expect("support");
                    let total = (self.original(part).0.len() - 1) as f64;
                    if first.0 == 0. && last.1 == total {
                        let a = root(&mut parent, first.2);
                        let b = root(&mut parent, last.2);
                        parent[a] = b;
                    }
                }
                let mut previous: Option<(f64, usize)> = None;
                for (lo, hi, id) in spans {
                    crate::cancel::check();
                    if let Some((end, before)) = previous
                        && lo <= end
                    {
                        let a = root(&mut parent, id);
                        let b = root(&mut parent, before);
                        parent[a] = b;
                        previous = Some((end.max(hi), before));
                    } else {
                        previous = Some((hi, id));
                    }
                }
            }
            let mut components = BTreeMap::<_, Vec<_>>::new();
            for (local, &id) in ids.iter().enumerate() {
                crate::cancel::check();
                components
                    .entry(root(&mut parent, local))
                    .or_default()
                    .push(id);
            }
            let contacts = junctions.entry(parts).or_default();
            contacts.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
            contacts.dedup();
            let contact_index = index::build(contacts.iter().enumerate().map(|(i, p)| {
                let c = Coord { x: p[0], y: p[1] };
                (i, geo::Rect::new(c, c))
            }));
            for component in components.into_values() {
                crate::cancel::check();
                let spans = [0, 1].map(|side| {
                    super::matching::union(
                        component
                            .iter()
                            .map(|&id| {
                                crate::cancel::check();
                                #[cfg(test)]
                                JUNCTION_SUPPORT_WORK.with(|n| n.set(n.get() + 1));
                                let source = matches[id].sources[side];
                                evidence[id].support[side].map(|p| along(source, p))
                            })
                            .collect(),
                    )
                });
                // A longer output on one representative cannot manufacture
                // longitudinal support missing from the other original passage.
                let short = [0, 1].iter().any(|&side| {
                    spans[side].iter().map(|s| s[1] - s[0]).sum::<f64>()
                        + 4. * uncertainty[parts[side]] * spans[side].len() as f64
                        <= self.tolerance
                });
                let exact = [0, 1].iter().all(|&side| {
                    let spans = super::matching::union(
                        component
                            .iter()
                            .filter(|&&id| evidence[id].exact)
                            .map(|&id| {
                                evidence[id].support[side]
                                    .map(|p| along(matches[id].sources[side], p))
                            })
                            .collect(),
                    );
                    spans.iter().map(|s| s[1] - s[0]).sum::<f64>()
                        - 4. * uncertainty[parts[side]] * spans.len() as f64
                        > self.tolerance
                });
                let bounds = [0, 1].map(|side| {
                    let s = &spans[side];
                    if !self.parts[parts[side]].is_closed {
                        return [s[0][0], s.last().expect("support")[1]];
                    }
                    let total = *positions[parts[side]].last().expect("source");
                    // Cut the cycle in its largest unsupported gap. A run
                    // across the source seam then has the same bounds after rotation.
                    let mut gap = s[0][0] + total - s.last().expect("support")[1];
                    let mut cut = s[0][0];
                    for pair in s.windows(2) {
                        let candidate = pair[1][0] - pair[0][1];
                        if candidate > gap {
                            gap = candidate;
                            cut = pair[1][0];
                        }
                    }
                    [cut, cut + total - gap]
                });
                let mut confined = false;
                if !short && !exact {
                    for side in 0..2 {
                        let margin = 4. * uncertainty[parts[side]];
                        let low = bounds[side][1] - self.tolerance + margin;
                        let high = bounds[side][0] + self.tolerance - margin;
                        if low > high {
                            continue;
                        }
                        let other = 1 - side;
                        let total = *positions[parts[other]].last().expect("source");
                        let side_total = *positions[parts[side]].last().expect("source");
                        let offsets: &[f64] = if self.parts[parts[side]].is_closed {
                            &[0., -side_total, side_total]
                        } else {
                            &[0.]
                        };
                        for &offset in offsets {
                            let a_low = (low + offset).max(0.);
                            let b_high = (high + offset).min(side_total);
                            if a_low > b_high {
                                continue;
                            }
                            let mut a = Coord { x: 0., y: 0. };
                            let mut b = Coord { x: 0., y: 0. };
                            if side == 0 {
                                a.x = a_low;
                                b.x = b_high;
                                b.y = total;
                            } else {
                                a.y = a_low;
                                b.y = b_high;
                                b.x = total;
                            }
                            confined = contact_index
                                .locate_in_envelope_intersecting(&index::envelope(geo::Rect::new(
                                    a, b,
                                )))
                                .any(|item| {
                                    crate::cancel::check();
                                    let mut p = contacts[item.data][other];
                                    if self.parts[parts[other]].is_closed && p < bounds[other][0] {
                                        p += total;
                                    }
                                    // Shared support continuing on both sides of a
                                    // junction is a run, rather than a one-sided cap.
                                    p < bounds[other][0] - 4. * uncertainty[parts[other]]
                                        || p > bounds[other][1] + 4. * uncertainty[parts[other]]
                                });
                            if confined {
                                break;
                            }
                        }
                        if confined {
                            break;
                        }
                    }
                }
                if short || confined {
                    for &id in &component {
                        rejected[id] = true;
                    }
                }
            }
        }
        self.retain_connected_part_support(matches, evidence, &mut rejected);
        for (m, rejected) in matches.iter_mut().zip(rejected) {
            m.overlap = !rejected;
        }
    }
    /// Several source parts can contribute one connected shared passage.
    /// Restore those contributions only when both features supply a long run
    /// without relying on the endpoint caps used for output geometry.
    fn retain_connected_part_support(
        &self,
        matches: &[Matched],
        evidence: &[super::matching::Evidence],
        rejected: &mut [bool],
    ) {
        if !rejected.iter().any(|&r| r) {
            return;
        }
        let key = |m: &Matched| {
            m.sources
                .map(|s| self.parts[self.segment(s).part].reference.feature())
        };
        let mut groups = BTreeMap::<_, Vec<_>>::new();
        for (i, m) in matches.iter().enumerate() {
            crate::cancel::check();
            if rejected[i] {
                groups.entry(key(m)).or_default();
            }
        }
        for (i, m) in matches.iter().enumerate() {
            crate::cancel::check();
            if let Some(ids) = groups.get_mut(&key(m)) {
                ids.push(i);
            }
        }
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for ids in groups.into_values() {
            crate::cancel::check();
            let participants = ids
                .iter()
                .flat_map(|&id| matches[id].sources)
                .map(|source| self.segment(source).part)
                .collect::<std::collections::BTreeSet<_>>();
            // Two parts cannot acquire additional multipart support. In particular,
            // sparse independent feature pairs need no passage-role allocations.
            if participants.len() <= 2 {
                continue;
            }
            let self_group = key(&matches[ids[0]])[0] == key(&matches[ids[0]])[1];
            let mut roles = BTreeMap::new();
            let mut ignored = std::collections::BTreeSet::new();
            if self_group {
                // Source-index order is not a passage role. Infer opposite
                // passages from correspondence, prioritizing nearby evidence.
                // A part can take different roles in disconnected source runs.
                // Index connected uncapped intervals, not the complete part path.
                let mut run_parent = (0..2 * ids.len()).collect::<Vec<_>>();
                let mut runs = BTreeMap::<_, Vec<_>>::new();
                for (local, &id) in ids.iter().enumerate() {
                    for side in 0..2 {
                        let source = self.segment(matches[id].sources[side]);
                        let span = evidence[id].support[side];
                        runs.entry(source.part).or_default().push((
                            source.start as f64 + span[0],
                            source.start as f64 + span[1],
                            2 * local + side,
                        ));
                    }
                }
                for (part, mut spans) in runs {
                    spans.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                    if self.parts[part].is_closed {
                        let first = spans.first().expect("support");
                        let last = spans
                            .iter()
                            .max_by(|a, b| a.1.total_cmp(&b.1))
                            .expect("support");
                        if first.0 == 0. && last.1 == (self.original(part).0.len() - 1) as f64 {
                            let a = root(&mut run_parent, first.2);
                            let b = root(&mut run_parent, last.2);
                            run_parent[a] = b;
                        }
                    }
                    let mut previous: Option<(f64, usize)> = None;
                    for (lo, hi, local) in spans {
                        crate::cancel::check();
                        if let Some((end, before)) = previous
                            && lo <= end
                        {
                            let a = root(&mut run_parent, local);
                            let b = root(&mut run_parent, before);
                            run_parent[a] = b;
                            previous = Some((end.max(hi), before));
                        } else {
                            previous = Some((hi, local));
                        }
                    }
                }
                let mut run_ids = BTreeMap::new();
                let mut passages = BTreeMap::new();
                for (local, &id) in ids.iter().enumerate() {
                    for side in 0..2 {
                        let run = root(&mut run_parent, 2 * local + side);
                        let next = run_ids.len();
                        passages.insert((id, side), *run_ids.entry(run).or_insert(next));
                    }
                }
                #[cfg(test)]
                PASSAGE_ROLE_SLOTS.with(|n| n.set(n.get() + run_ids.len()));
                let mut passage_parent = (0..run_ids.len()).collect::<Vec<_>>();
                let mut parity = vec![false; run_ids.len()];
                let mut rank = vec![0usize; run_ids.len()];
                fn passage(parent: &[usize], parity: &[bool], mut i: usize) -> (usize, bool) {
                    let mut side = false;
                    while parent[i] != i {
                        side ^= parity[i];
                        i = parent[i];
                    }
                    (i, side)
                }
                let mut support = BTreeMap::<_, BTreeMap<_, Vec<_>>>::new();
                for &id in &ids {
                    if !rejected[id] {
                        continue;
                    }
                    for side in 0..2 {
                        support
                            .entry(passages[&(id, side)])
                            .or_default()
                            .entry(matches[id].sources[side])
                            .or_default()
                            .push(evidence[id].support[side]);
                    }
                }
                let lengths = support
                    .into_iter()
                    .map(|(passage, sources)| {
                        let length = sources
                            .into_iter()
                            .map(|(source, spans)| {
                                let line = self.source(source);
                                super::matching::union(spans)
                                    .iter()
                                    .map(|span| span[1] - span[0])
                                    .sum::<f64>()
                                    * distance(line.start, line.end)
                            })
                            .sum::<f64>();
                        (passage, length)
                    })
                    .collect::<BTreeMap<_, _>>();
                let score = |id| {
                    [0, 1]
                        .map(|side| lengths.get(&passages[&(id, side)]).copied().unwrap_or(0.))
                        .into_iter()
                        .fold(0., f64::max)
                };
                let geometry_key = |id: usize| {
                    let mut pair = [0, 1].map(|side| {
                        let source = matches[id].sources[side];
                        let mut points = evidence[id].support[side]
                            .map(|t| super::matching::point(self.source(source), t));
                        if coordinate_order(&points[0], &points[1]).is_gt() {
                            points.swap(0, 1);
                        }
                        points
                    });
                    let cmp = |a: &[Coord; 2], b: &[Coord; 2]| {
                        coordinate_order(&a[0], &b[0]).then(coordinate_order(&a[1], &b[1]))
                    };
                    if cmp(&pair[0], &pair[1]).is_gt() {
                        pair.swap(0, 1);
                    }
                    pair
                };
                let mut ordered = ids.clone();
                ordered.sort_unstable_by(|&a, &b| {
                    (!rejected[a])
                        .cmp(&(!rejected[b]))
                        .then(score(b).total_cmp(&score(a)))
                        .then_with(|| {
                            let a = geometry_key(a);
                            let b = geometry_key(b);
                            a.into_iter()
                                .flatten()
                                .zip(b.into_iter().flatten())
                                .map(|(a, b)| coordinate_order(&a, &b))
                                .find(|o| !o.is_eq())
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                });
                for id in ordered {
                    crate::cancel::check();
                    let parts = [passages[&(id, 0)], passages[&(id, 1)]];
                    let (a, pa) = passage(&passage_parent, &parity, parts[0]);
                    let (b, pb) = passage(&passage_parent, &parity, parts[1]);
                    if a == b {
                        if pa == pb {
                            // Keep the certified decision on a contradictory
                            // correspondence. Compatible connected support can
                            // still establish a different sustained passage.
                            ignored.insert(id);
                        }
                    } else if rank[a] < rank[b] {
                        passage_parent[a] = b;
                        parity[a] = pa ^ pb ^ true;
                    } else {
                        passage_parent[b] = a;
                        parity[b] = pa ^ pb ^ true;
                        if rank[a] == rank[b] {
                            rank[a] += 1;
                        }
                    }
                }
                for &id in &ids {
                    for side in 0..2 {
                        let (_, role) = passage(&passage_parent, &parity, passages[&(id, side)]);
                        roles.insert((id, side), usize::from(role));
                    }
                }
            }
            let connected = |ids: &[usize]| {
                let mut parent = [0, 1].map(|_| (0..ids.len()).collect::<Vec<_>>());
                let mut spans = BTreeMap::<_, Vec<_>>::new();
                let mut nodes = BTreeMap::new();
                for side in 0..2 {
                    for (local, &id) in ids.iter().enumerate() {
                        crate::cancel::check();
                        if ignored.contains(&id) {
                            continue;
                        }
                        let source = self.segment(matches[id].sources[side]);
                        let role = if self_group { roles[&(id, side)] } else { side };
                        let span = evidence[id].support[side];
                        spans.entry((role, source.part)).or_default().push((
                            source.start as f64 + span[0],
                            source.start as f64 + span[1],
                            local,
                        ));
                        for t in span {
                            let p =
                                super::matching::point(self.source(matches[id].sources[side]), t);
                            let k = (role, super::cluster::bits(p.x), super::cluster::bits(p.y));
                            if let Some(&(part, before)) = nodes.get(&k) {
                                if part != source.part {
                                    let a = root(&mut parent[role], local);
                                    let b = root(&mut parent[role], before);
                                    parent[role][a] = b;
                                }
                            } else {
                                nodes.insert(k, (source.part, local));
                            }
                        }
                    }
                }
                for ((role, part), mut spans) in spans {
                    spans.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                    if self.parts[part].is_closed {
                        let first = spans.first().expect("support");
                        let last = spans
                            .iter()
                            .max_by(|a, b| a.1.total_cmp(&b.1))
                            .expect("support");
                        if first.0 == 0. && last.1 == (self.original(part).0.len() - 1) as f64 {
                            let a = root(&mut parent[role], first.2);
                            let b = root(&mut parent[role], last.2);
                            parent[role][a] = b;
                        }
                    }
                    let mut previous: Option<(f64, usize)> = None;
                    for (lo, hi, id) in spans {
                        crate::cancel::check();
                        if let Some((end, before)) = previous
                            && lo <= end
                        {
                            let a = root(&mut parent[role], id);
                            let b = root(&mut parent[role], before);
                            parent[role][a] = b;
                            previous = Some((end.max(hi), before));
                        } else {
                            previous = Some((hi, id));
                        }
                    }
                }
                let mut components = BTreeMap::<_, Vec<_>>::new();
                for (local, &id) in ids.iter().enumerate() {
                    crate::cancel::check();
                    if ignored.contains(&id) {
                        continue;
                    }
                    components
                        .entry([root(&mut parent[0], local), root(&mut parent[1], local)])
                        .or_default()
                        .push(id);
                }
                components.into_values().collect::<Vec<_>>()
            };
            let mut pending = connected(&ids);
            while let Some(component) = pending.pop() {
                crate::cancel::check();
                if !component.iter().any(|&id| rejected[id]) {
                    continue;
                }
                // Connectivity must hold inside the accepted correspondence group.
                // A bridge excluded by the opposite role cannot connect its support.
                let subdivisions = connected(&component);
                if subdivisions.len() > 1 {
                    pending.extend(subdivisions);
                    continue;
                }

                crate::cancel::check();
                let mut multipart = false;
                let sustained = [0, 1].iter().all(|&side| {
                    use super::certified::Interval;
                    let mut by_source = BTreeMap::<_, Vec<_>>::new();
                    let mut parts = std::collections::BTreeSet::new();
                    for &id in &component {
                        crate::cancel::check();
                        for original_side in 0..2 {
                            let source = matches[id].sources[original_side];
                            let part = self.segment(source).part;
                            let role = if self_group {
                                roles[&(id, original_side)]
                            } else {
                                original_side
                            };
                            if role == side {
                                parts.insert(part);
                                by_source
                                    .entry(source)
                                    .or_default()
                                    .push(evidence[id].support[original_side]);
                            }
                        }
                    }
                    let multiple_parts = parts.len() > 1;
                    let mut part_lengths = BTreeMap::<usize, Interval>::new();
                    // Original supporting-line identities let repeated and
                    // unequally segmented parts contribute their geometric union.
                    let mut physical = BTreeMap::<_, Vec<_>>::new();
                    for (source, spans) in by_source {
                        crate::cancel::check();
                        let line = self.source(source);
                        let x = Interval(line.end.x, line.end.x)
                            .sub(Interval(line.start.x, line.start.x));
                        let y = Interval(line.end.y, line.end.y)
                            .sub(Interval(line.start.y, line.start.y));
                        let length = x.square().add(y.square()).sqrt();
                        let spans = super::matching::union(spans);
                        let part = self.segment(source).part;
                        let total = part_lengths.entry(part).or_insert(Interval(0., 0.));
                        for span in &spans {
                            *total =
                                total.add(length.mul(
                                    Interval(span[1], span[1]).sub(Interval(span[0], span[0])),
                                ));
                        }
                        let q = |v: f64| {
                            BigRational::from_float(v).expect("finite original coordinate")
                        };
                        let x0 = q(line.start.x);
                        let y0 = q(line.start.y);
                        let dx = q(line.end.x) - &x0;
                        let dy = q(line.end.y) - &y0;
                        let vertical = dx == BigRational::from_integer(0.into());
                        let slope = if vertical {
                            BigRational::from_integer(0.into())
                        } else {
                            &dy / &dx
                        };
                        let intercept = if vertical {
                            x0.clone()
                        } else {
                            &y0 - &slope * &x0
                        };
                        let (origin, delta) = if vertical { (y0, dy) } else { (x0, dx) };
                        for span in spans {
                            if !span.iter().all(|v| v.is_finite()) {
                                return true;
                            }
                            let a = &origin + q(span[0]) * &delta;
                            let b = &origin + q(span[1]) * &delta;
                            physical
                                .entry((vertical, slope.clone(), intercept.clone()))
                                .or_default()
                                .push(if a <= b { [a, b] } else { [b, a] });
                        }
                    }
                    let mut total = Interval(0., 0.);
                    for ((vertical, slope, _), mut spans) in physical {
                        crate::cancel::check();
                        spans.sort_unstable_by(|a, b| a[0].cmp(&b[0]));
                        let mut merged: Vec<[BigRational; 2]> = Vec::new();
                        for span in spans {
                            if let Some(last) = merged.last_mut()
                                && span[0] <= last[1]
                            {
                                last[1] = last[1].clone().max(span[1].clone());
                            } else {
                                merged.push(span);
                            }
                        }
                        let ratio = if vertical {
                            Interval(1., 1.)
                        } else {
                            let Some(slope) = slope.to_f64().filter(|v| v.is_finite()) else {
                                return true;
                            };
                            Interval(slope.next_down(), slope.next_up())
                                .square()
                                .add(Interval(1., 1.))
                                .sqrt()
                        };
                        for span in merged {
                            let Some(width) =
                                (&span[1] - &span[0]).to_f64().filter(|v| v.is_finite())
                            else {
                                return true;
                            };
                            total =
                                total.add(Interval(width.next_down(), width.next_up()).mul(ratio));
                        }
                    }
                    // Duplicate or contained parts add no connected physical support.
                    // Restoration requires a certified contribution beyond every
                    // individual part, not merely a second part identity.
                    let longest = part_lengths
                        .values()
                        .map(|length| length.1)
                        .fold(0., f64::max);
                    multipart |= multiple_parts && total.0 > longest;
                    // Inconclusive arithmetic preserves the original evidence.
                    total.1 > self.tolerance
                });
                if multipart && sustained {
                    for id in component {
                        rejected[id] = false;
                    }
                }
            }
        }
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
