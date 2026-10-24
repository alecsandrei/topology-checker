//! The construction and matching the faster ones replaced, kept to check that
//! they give the same graphs and violations.
use super::*;
use crate::{
    CancellationToken,
    engine::{cluster::Cluster, index},
};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicUsize, Ordering},
};

/// Requests whose graphs are built the old way.
static TOKENS: Mutex<Vec<CancellationToken>> = Mutex::new(Vec::new());
static COUNT: AtomicUsize = AtomicUsize::new(0);
/// Whether graphs built now serve a request registered with [`Old::new`].
pub(super) fn active() -> bool {
    if COUNT.load(Ordering::Acquire) == 0 {
        return false;
    }
    let Some(token) = crate::cancel::current() else {
        return false;
    };
    TOKENS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|t| t.same(&token))
}
/// Graphs of validations under a token are built the old way while this lives.
pub(super) struct Old(CancellationToken);
impl Old {
    pub(super) fn new(token: &CancellationToken) -> Self {
        TOKENS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(token.clone());
        COUNT.fetch_add(1, Ordering::Release);
        Self(token.clone())
    }
}
impl Drop for Old {
    fn drop(&mut self) {
        let mut tokens = TOKENS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = tokens.iter().position(|t| t.same(&self.0)) {
            tokens.swap_remove(i);
            COUNT.fetch_sub(1, Ordering::Release);
        }
    }
}

fn trim_returns(
    parts: &mut [GraphPart],
    raw_edges: &mut Vec<([usize; 2], EdgeUse)>,
    trims: Vec<Trim>,
    ties: &HashMap<(usize, bool), Vec<Coord>>,
    node: impl Fn(Coord) -> Option<usize>,
) {
    // Removed visits before the kept start and after the kept end, without
    // the end visits themselves.
    let removed = |trim: &Trim, n: usize| {
        [
            1..trim.bounds[0].max(1),
            (trim.bounds[1] + 1).min(n - 1)..n - 1,
        ]
    };
    // Parts ending at a removed node.
    let mut ends = HashMap::<usize, Vec<usize>>::new();
    for trim in &trims {
        let nodes = &parts[trim.part].nodes;
        for range in removed(trim, nodes.len()) {
            for &node in &nodes[range] {
                ends.entry(node).or_default();
            }
        }
    }
    for (id, part) in parts.iter().enumerate() {
        if part.is_closed {
            continue;
        }
        for node in [part.nodes.first(), part.nodes.last()]
            .into_iter()
            .flatten()
        {
            if let Some(ends) = ends.get_mut(node)
                && !ends.contains(&id)
            {
                ends.push(id);
            }
        }
    }
    let mut dropped = Vec::<usize>::new();
    for trim in trims {
        let nodes = &parts[trim.part].nodes;
        let n = nodes.len();
        let [mut start, mut end] = removed(&trim, n);
        let [first, last] = [nodes[0], nodes[n - 1]];
        let own = |i: &usize| ends[&nodes[*i]].iter().all(|&p| p == trim.part);
        let tied = |end: bool, at: usize| {
            ties.get(&(trim.part, end))
                .is_some_and(|points| points.iter().any(|&p| node(p) != Some(at)))
        };
        let mut bounds = [0, n - 1];
        if !tied(true, last) && end.all(|i| nodes[i] == last || (nodes[i] != first && own(&i))) {
            bounds[1] = trim.bounds[1];
            dropped.extend(trim.edges[bounds[1] + 1..].iter().flatten());
        }
        if !tied(false, first) && start.all(|i| nodes[i] == first || (nodes[i] != last && own(&i)))
        {
            bounds[0] = trim.bounds[0];
            dropped.extend(trim.edges[1..=bounds[0]].iter().flatten());
        }
        if bounds != [0, n - 1] {
            parts[trim.part].nodes = nodes[bounds[0]..=bounds[1]].to_vec();
        }
    }
    dropped.sort_unstable();
    let mut next = dropped.into_iter().peekable();
    let mut index = 0;
    raw_edges.retain(|_| {
        let keep = next.next_if_eq(&index).is_none();
        index += 1;
        keep
    });
}
fn add_cut(cuts: &mut HashMap<usize, Vec<Coord>>, source: usize, line: Line, point: Coord) {
    if point != line.start && point != line.end {
        cuts.entry(source).or_default().push(point);
    }
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
    pub(super) fn reference_matches(&self) -> Vec<Matched> {
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
                crate::engine::matching::evidence(self.source(a), self.source(b), self.tolerance)
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
            self.reference_retain(&tree, &mut result, &evidence);
        }
        result.sort_unstable_by_key(|m| m.sources);
        result
    }

    /// Filter connected original support, before output endpoint caps are used.
    /// A point junction can cover a short passage, but cannot establish overlap.
    fn reference_retain(
        &self,
        tree: &index::Index,
        matches: &mut [Matched],
        evidence: &[crate::engine::matching::Evidence],
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
            // Passages of one part that meet through a loop are judged as runs too.
            if !self.consecutive(m.sources[0], m.sources[1]) && (!evidence[i].exact || short) {
                groups.entry(parts).or_default();
            }
        }
        if groups.is_empty() {
            return;
        }
        for (i, m) in matches.iter().enumerate() {
            crate::cancel::check();
            let parts = m.sources.map(|s| self.segment(s).part);
            if !self.consecutive(m.sources[0], m.sources[1])
                && let Some(ids) = groups.get_mut(&parts)
            {
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
                let mut bounds = crate::engine::certified::Interval(0., 0.);
                positions[part].push(total);
                for line in self.original(part).lines() {
                    crate::cancel::check();
                    #[cfg(test)]
                    JUNCTION_SUPPORT_WORK.with(|n| n.set(n.get() + 1));
                    total += distance(line.start, line.end);
                    use crate::engine::certified::Interval;
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
        let mut meetings = BTreeMap::<_, Vec<Meeting>>::new();
        for (a, b) in index::pairs(tree, tree, self.tolerance, true) {
            crate::cancel::check();
            #[cfg(test)]
            JUNCTION_SUPPORT_WORK.with(|n| n.set(n.get() + 1));
            let parts = [self.segment(a).part, self.segment(b).part];
            if !groups.contains_key(&parts) || self.consecutive(a, b) {
                continue;
            }
            let lines = [self.source(a), self.source(b)];
            let intersection = line_intersection(lines[0], lines[1]);
            // Points where the lines meet: they intersect or touch, or come within
            // a small fraction of the tolerance at a vertex, as a junction vertex
            // stored with rounding differences or a small digitizing gap does.
            let mut points = match intersection {
                Some(LineIntersection::SinglePoint { intersection, .. }) => vec![intersection],
                Some(LineIntersection::Collinear { intersection }) => {
                    vec![intersection.start, intersection.end]
                }
                None => Vec::new(),
            };
            for side in 0..2 {
                for p in [lines[side].start, lines[side].end] {
                    if distance(p, projection(lines[1 - side], p).1) <= self.tolerance / 16. {
                        points.push(p);
                    }
                }
            }
            for p in points {
                meetings.entry(parts).or_default().push(Meeting {
                    positions: [
                        along(a, projection(lines[0], p).0),
                        along(b, projection(lines[1], p).0),
                    ],
                    end: None,
                });
            }
            if let Some(LineIntersection::SinglePoint {
                intersection,
                is_proper: false,
            }) = intersection
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
        // Several segment pairs report the same meeting point.
        for list in meetings.values_mut() {
            list.sort_by(|a, b| {
                (a.positions[0].total_cmp(&b.positions[0]))
                    .then(a.positions[1].total_cmp(&b.positions[1]))
            });
            list.dedup_by(|a, b| a.positions == b.positions);
        }
        let mut rejected = vec![false; matches.len()];
        let mut explained = vec![false; matches.len()];
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
            let mut ends_met = false;
            for component in components.into_values() {
                crate::cancel::check();
                let spans = [0, 1].map(|side| {
                    crate::engine::matching::union(
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
                    let spans = crate::engine::matching::union(
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
                // Lines that meet at an angle stay within tolerance near the
                // meeting point. Only support beyond those near zones can make a
                // run shared linework. Without a meeting point nothing changes.
                let near = !short && !exact && {
                    // Line ends within tolerance of the other part meet it too.
                    if !ends_met {
                        let ends = self.end_meetings(tree, parts, &positions);
                        meetings.entry(parts).or_default().extend(ends);
                        ends_met = true;
                    }
                    self.near_zones_explain(
                        tree,
                        parts,
                        &positions,
                        &uncertainty,
                        &spans,
                        &meetings[&parts],
                    )
                };
                // Within one part, only a meeting point can bound a run.
                let judged = parts[0] != parts[1] && (short || confined);
                if judged || near {
                    for &id in &component {
                        rejected[id] = true;
                        // Other parts cannot restore what a meeting point explains.
                        explained[id] = near;
                    }
                }
            }
        }
        self.retain_connected_part_support(matches, evidence, &mut rejected, &explained);
        for (m, rejected) in matches.iter_mut().zip(rejected) {
            m.overlap = !rejected;
        }
    }
    pub(super) fn reference_build(
        t: &Topology,
        layers: &[(LayerId, InputMode)],
    ) -> Result<Self, Error> {
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
        // A point is on a feature's boundary when an odd number of its open parts end
        // there (the OGC mod-2 rule), counted on original feature coordinates. Closed
        // parts and passages add nothing; nearby clustered endpoints stay separate.
        let mut endpoint_counts = HashMap::<_, usize>::new();
        // The same ends by feature and coordinate, to find those beside a contact.
        let mut feature_ends = Vec::with_capacity(endpoints.len());
        for (&point, &source) in endpoints.iter().zip(&endpoint_sources) {
            let feature = parts[segments[source].part].reference.feature();
            *endpoint_counts
                .entry((
                    feature,
                    crate::engine::cluster::bits(point.x),
                    crate::engine::cluster::bits(point.y),
                ))
                .or_default() += 1;
            // Adding zero makes negative zeros positive, so the order agrees with the
            // comparisons of the search.
            let end = Coord {
                x: point.x + 0.,
                y: point.y + 0.,
            };
            feature_ends.push((feature, end));
        }
        feature_ends.sort_by(|a, b| a.0.cmp(&b.0).then(coordinate_order(&a.1, &b.1)));
        feature_ends.dedup();
        let mut original_contacts = Vec::new();
        let mut contact = |a: usize, b: usize, points: [Coord; 2], coordinate: Coord| {
            let sources = [a, b];
            let [a, b] = sources.map(|source| segments[source].part);
            if parts[a].reference.feature() == parts[b].reference.feature() {
                return;
            }
            // The nearest end of the whole feature decides, and an end at the contact
            // itself is nearest. Otherwise the touched part's own ends count within
            // tolerance; other parts' ends only where they coincide with the contact
            // up to rounding of the coordinates the contact came from, so within twice
            // their rounding reach.
            let [l0, l1] = sources.map(|source| segments[source].line(&parts));
            let interior = [0, 1].map(|side| {
                let p = &parts[[a, b][side]];
                let feature = p.reference.feature();
                let point = points[side];
                let count = |end: Coord| {
                    let key = (
                        feature,
                        crate::engine::cluster::bits(end.x),
                        crate::engine::cluster::bits(end.y),
                    );
                    endpoint_counts.get(&key).copied()
                };
                let nearest = || {
                    let own = [p.original.0[0], *p.original.0.last().expect("source")]
                        .into_iter()
                        .filter(|&end| !p.closed() && distance(point, end) <= t.tolerance());
                    // Rounding is the largest coordinate's, so an end only adds its own.
                    let base = rounding(&[point, l0.start, l0.end, l1.start, l1.end]);
                    let window = 2. * base.min(t.tolerance());
                    let others = ends_near(&feature_ends, feature, point, window).filter(|&end| {
                        distance(point, end) <= base.max(rounding(&[end])).min(t.tolerance())
                    });
                    own.chain(others).min_by(|a, b| {
                        distance(point, *a)
                            .total_cmp(&distance(point, *b))
                            .then(coordinate_order(a, b))
                    })
                };
                count(point)
                    .or_else(|| nearest().and_then(count))
                    .is_none_or(|count| count.is_multiple_of(2))
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
        // Exact intersections meet a vertex lying on another segment only where rounding
        // leaves it on or across that segment. A vertex within rounding of a segment, and
        // within tolerance, meets it from either side, as in data stored without that
        // rounding. A proper intersection beside such a segment end is the same meeting,
        // so it is recorded there: its own rounded coordinate could join another node.
        let tolerance = t.tolerance();
        let near = |vertex: Coord, segment: Line| {
            let (_, projected) = projection(segment, vertex);
            (tolerance > 0.
                && distance(vertex, projected)
                    <= rounding(&[vertex, segment.start, segment.end]).min(tolerance))
            .then_some(projected)
        };
        // Interior vertices, including ring seams. Part endpoints are cracked instead.
        let vertex = |s: &Segment| {
            let part = &parts[s.part];
            (s.start > 0 || part.closed()).then(|| part.original.0[s.start])
        };
        let reach = rounding(&coordinates).min(tolerance);
        for (i, s) in segments.iter().enumerate() {
            crate::cancel::check();
            let line = s.line(&parts);
            for item in
                tree.locate_in_envelope_intersecting(&index::expanded(line.bounding_rect(), reach))
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
                            // The nearest segment end within rounding of the other segment.
                            let beside = [line.start, line.end, other.start, other.end]
                                .into_iter()
                                .enumerate()
                                .filter(|&(_, p)| is_proper && distance(p, point) <= tolerance)
                                .filter_map(|(k, p)| {
                                    Some(if k < 2 {
                                        (p, [p, near(p, other)?])
                                    } else {
                                        (p, [near(p, line)?, p])
                                    })
                                })
                                .min_by(|a, b| {
                                    distance(a.0, point).total_cmp(&distance(b.0, point))
                                });
                            match beside {
                                Some((p, points)) => contact(i, j, points, p),
                                None => contact(i, j, [point, point], point),
                            }
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
                if let Some(v) = vertex(s)
                    && let Some(projected) = near(v, other)
                {
                    contact(i, j, [v, projected], v);
                }
                if let Some(v) = vertex(&segments[j])
                    && let Some(projected) = near(v, line)
                {
                    contact(i, j, [projected, v], v);
                }
            }
        }
        let mut cracked = Vec::new();
        // Crack points on other parts, for line ends also near their own line.
        let mut ties = HashMap::<(usize, bool), Vec<Coord>>::new();
        // Endpoint cracking has no effect on the immutable input geometries.
        if t.tolerance() > 0. {
            for (&point, &source) in endpoints.iter().zip(&endpoint_sources) {
                let mut candidates = tree
                    .locate_in_envelope_intersecting(&index::at(point, t.tolerance()))
                    .map(|i| i.data)
                    .collect::<Vec<_>>();
                candidates.sort_unstable();
                let part = segments[source].part;
                let (mut own, mut others) = (false, Vec::new());
                for i in candidates {
                    let line = segments[i].line(&parts);
                    let (_, projected) = projection(line, point);
                    if distance(point, projected) <= t.tolerance() {
                        contact(source, i, [point, projected], point);
                        add_cut(&mut split, i, line, projected);
                        cracked.push(projected);
                        if segments[i].part == part {
                            own |= i != source;
                        } else {
                            others.push(projected);
                        }
                    }
                }
                if own && !others.is_empty() {
                    let last = point == *parts[part].original.0.last().expect("source");
                    ties.insert((part, last), others);
                }
            }
        }
        // Endpoint parity is construction evidence, not retained graph storage.
        // Release it before allocating noded edges and incidences.
        drop(endpoint_counts);
        drop(feature_ends);
        drop(endpoint_sources);
        let order = tree.iter().map(|i| u32::try_from(i.data).ok()).collect();
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
        // Segments of one part are consecutive, so one path is built at a time.
        let mut path: Vec<Visit> = Vec::new();
        let mut trims = Vec::new();
        for (source, s) in segments.iter().enumerate() {
            crate::cancel::check();
            let line = s.line(&parts);
            let mut cuts = split.remove(&source).unwrap_or_default();
            cuts.extend([line.start, line.end]);
            cuts.sort_by(|a, b| parameter(line, *a).total_cmp(&parameter(line, *b)));
            cuts.dedup();
            let mut previous = None;
            for point in cuts {
                let node = if let Some(indexed) = &indexed {
                    indexed.node(point)
                } else {
                    cluster.as_mut().expect("tolerance cluster").insert(point)
                };
                let at = (point, s.start);
                if previous == Some(node) {
                    if let Some(visit) = path.last_mut() {
                        visit.last = at;
                    }
                    continue;
                }
                let edge = previous.map(|p: usize| {
                    (
                        [p.min(node), p.max(node)],
                        EdgeUse {
                            part: s.part,
                            source,
                        },
                    )
                });
                previous = Some(node);
                match path.last_mut() {
                    Some(visit) if visit.node == node => visit.last = at,
                    _ => path.push(Visit {
                        node,
                        first: at,
                        last: at,
                        edge,
                    }),
                }
            }
            if segments
                .get(source + 1)
                .is_some_and(|next| next.part == s.part)
            {
                continue;
            }
            let part = &mut parts[s.part];
            let bounds = if part.is_closed || t.tolerance() == 0. {
                [0, path.len() - 1]
            } else {
                kept_visits(&path, &part.original.0, t.tolerance())
            };
            part.nodes.extend(path.iter().map(|v| v.node));
            if bounds == [0, path.len() - 1] {
                raw_edges.extend(path.iter().filter_map(|v| v.edge));
            } else {
                let edges = path
                    .iter()
                    .map(|v| {
                        v.edge.map(|edge| {
                            raw_edges.push(edge);
                            raw_edges.len() - 1
                        })
                    })
                    .collect();
                trims.push(Trim {
                    part: s.part,
                    bounds,
                    edges,
                });
            }
            path.clear();
        }
        if !trims.is_empty() {
            let cluster = cluster.as_ref().expect("tolerance cluster");
            trim_returns(&mut parts, &mut raw_edges, trims, &ties, |p| cluster.get(p));
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
            contact_nodes: Cache::new(),
            coords,
            sources: segments,
            parts,
            incident,
            contacts,
            order,
            reference: true,
        })
    }
}

mod tests {
    use super::*;
    use crate::{
        Layer, Rule, SinkError,
        cancel::{self, hooks},
    };
    use geo::{Geometry, MapCoords, MultiLineString, Polygon};
    use std::sync::Arc;

    struct Random(u64);
    impl Random {
        fn new(seed: u64) -> Self {
            Self(seed.wrapping_add(1).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
        }
        fn next(&mut self) -> u64 {
            let mut s = self.0;
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            self.0 = s;
            s
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn pick<T: Copy>(&mut self, items: &[T]) -> T {
            items[self.below(items.len())]
        }
    }

    /// Points of a coarse grid, so that lines often touch, cross and overlap,
    /// some moved within or just beyond the tolerance.
    struct Grid {
        tolerance: f64,
        offset: f64,
        unit: f64,
        size: usize,
    }
    impl Grid {
        fn value(&self, r: &mut Random) -> f64 {
            let v = r.below(self.size + 1) as f64 * self.unit;
            let nudge = match r.below(10) {
                0 => self.tolerance / 2.,
                1 => -self.tolerance,
                2 => self.tolerance * 1.5,
                3 => 1e-12,
                _ => 0.,
            };
            if v == 0. && nudge == 0. && self.offset == 0. && r.below(2) == 0 {
                -0.
            } else {
                v + nudge + self.offset
            }
        }
        fn point(&self, r: &mut Random) -> Coord {
            Coord {
                x: self.value(r),
                y: self.value(r),
            }
        }
        fn chain(&self, r: &mut Random, from: Option<Coord>, n: usize) -> Vec<Coord> {
            let mut coords = vec![from.unwrap_or_else(|| self.point(r))];
            while coords.len() < n {
                let last = coords[coords.len() - 1];
                let next = if r.below(3) == 0 {
                    self.point(r)
                } else {
                    let mut step = || r.pick(&[-2., -1., 0., 1., 2.]) * self.unit;
                    Coord {
                        x: last.x + step(),
                        y: last.y + step(),
                    }
                };
                coords.push(next);
            }
            coords
        }
    }
    fn parts(geometry: &Geometry) -> Vec<LineString> {
        match geometry {
            Geometry::LineString(l) => vec![l.clone()],
            Geometry::MultiLineString(m) => m.0.clone(),
            _ => Vec::new(),
        }
    }
    fn lines(parts: Vec<LineString>) -> Geometry {
        if parts.len() == 1 {
            parts.into_iter().next().expect("part").into()
        } else {
            MultiLineString(parts).into()
        }
    }

    /// Line features: chains from earlier vertices, rings, multiparts,
    /// repeated, reversed and moved features, retraces, parts collapsing
    /// within tolerance and long runs along grid lines.
    fn features(grid: &Grid, r: &mut Random, count: usize, first: i64) -> Vec<(i64, Geometry)> {
        let mut out: Vec<(i64, Geometry)> = Vec::new();
        let mut vertices = Vec::new();
        let from = |r: &mut Random, vertices: &[Coord]| {
            (!vertices.is_empty() && r.below(2) == 0).then(|| r.pick(vertices))
        };
        for k in 0..count {
            let earlier = (!out.is_empty()).then(|| out[r.below(out.len())].1.clone());
            let geometry = match (r.below(12), earlier) {
                (6, Some(g)) => g,
                (7, Some(g)) => lines(
                    parts(&g)
                        .into_iter()
                        .rev()
                        .map(|l| LineString(l.0.into_iter().rev().collect()))
                        .collect(),
                ),
                (8, Some(g)) => {
                    let shift = r.pick(&[grid.tolerance / 2., grid.tolerance, 1e-12]);
                    let (dx, dy) = r.pick(&[(shift, 0.), (0., shift), (shift, -shift)]);
                    g.map_coords(|c| Coord {
                        x: c.x + dx,
                        y: c.y + dy,
                    })
                }
                (4, _) => {
                    let (start, n) = (from(r, &vertices), 3 + r.below(3));
                    let mut ring = grid.chain(r, start, n);
                    ring.push(ring[0]);
                    LineString(ring).into()
                }
                (5, _) => lines(
                    (0..2 + r.below(2))
                        .map(|_| {
                            let (start, n) = (from(r, &vertices), 2 + r.below(4));
                            LineString(grid.chain(r, start, n))
                        })
                        .collect(),
                ),
                (9, _) => {
                    let [a, b, c] = [0; 3].map(|_| grid.point(r));
                    LineString(if r.below(2) == 0 {
                        vec![a, b, a]
                    } else {
                        vec![a, b, c, b]
                    })
                    .into()
                }
                (10, _) => {
                    let p = grid.point(r);
                    let q = Coord {
                        x: p.x + grid.tolerance / 4.,
                        y: p.y,
                    };
                    lines(vec![
                        LineString(vec![p, q]),
                        LineString(grid.chain(r, Some(q), 3)),
                    ])
                }
                (11, _) => {
                    let p = grid.point(r);
                    let length = (1 + r.below(grid.size)) as f64 * grid.unit;
                    let q = if r.below(2) == 0 {
                        Coord {
                            x: p.x + length,
                            y: p.y,
                        }
                    } else {
                        Coord {
                            x: p.x,
                            y: p.y + length,
                        }
                    };
                    LineString(vec![p, q]).into()
                }
                _ => {
                    let (start, n) = (from(r, &vertices), 2 + r.below(5));
                    LineString(grid.chain(r, start, n)).into()
                }
            };
            vertices.extend(parts(&geometry).into_iter().flat_map(|l| l.0));
            out.push((first + k as i64 * 3, geometry));
        }
        out
    }
    /// Rectangles, some with holes, and triangles on line segments.
    fn polygons(
        grid: &Grid,
        r: &mut Random,
        count: usize,
        on: &[(i64, Geometry)],
    ) -> Vec<(i64, Geometry)> {
        let segments = on
            .iter()
            .flat_map(|(_, g)| parts(g))
            .flat_map(|l| l.lines().collect::<Vec<_>>())
            .filter(|l| l.dx().hypot(l.dy()) >= grid.unit / 2.)
            .collect::<Vec<_>>();
        (0..count)
            .map(|k| {
                let polygon = if r.below(2) == 0 && !segments.is_empty() {
                    let Line { start: a, end: b } = r.pick(&segments);
                    let side = r.pick(&[-1., 1.]);
                    let c = Coord {
                        x: (a.x + b.x) / 2. - side * (b.y - a.y),
                        y: (a.y + b.y) / 2. + side * (b.x - a.x),
                    };
                    Polygon::new(LineString(vec![a, b, c, a]), Vec::new())
                } else {
                    let a = grid.point(r);
                    let w = (1 + r.below(4)) as f64 * grid.unit;
                    let h = (1 + r.below(4)) as f64 * grid.unit;
                    let at = |x: f64, y: f64| Coord {
                        x: a.x + x,
                        y: a.y + y,
                    };
                    let ring = |x0: f64, y0: f64, x1: f64, y1: f64| {
                        LineString(vec![
                            at(x0, y0),
                            at(x1, y0),
                            at(x1, y1),
                            at(x0, y1),
                            at(x0, y0),
                        ])
                    };
                    let holes = if r.below(3) == 0 {
                        vec![ring(w / 4., h / 4., w / 2., h / 2.)]
                    } else {
                        Vec::new()
                    };
                    Polygon::new(ring(0., 0., w, h), holes)
                };
                (k as i64, polygon.into())
            })
            .collect()
    }

    /// Tolerance and layers of one case: lines, other lines and polygons.
    struct Case {
        tolerance: f64,
        layers: [Arc<Layer>; 3],
    }
    fn case(seed: u64, count: usize) -> Case {
        let mut r = Random::new(seed);
        let tolerance = r.pick(&[0., 0., 1e-9, 0.001, 0.05, 0.3, 1., 2.5]);
        let grid = Grid {
            tolerance,
            offset: r.pick(&[0., 0., 0., 1e6, -3.5e7, 1_099_511_627_776.]),
            unit: r.pick(&[1., 0.5, 0.25]),
            size: r.pick(&[4, 8, 16]),
        };
        let lines = features(&grid, &mut r, count, -40);
        let mut others = features(&grid, &mut r, count / 2, 5);
        // Linework shared by the layers.
        for k in 0..count / 4 {
            others.push((1000 + k as i64, lines[r.below(lines.len())].1.clone()));
        }
        let polygons = polygons(&grid, &mut r, 1 + count / 4, &lines);
        Case {
            tolerance,
            layers: [
                Arc::new(Layer::from_features("lines", lines)),
                Arc::new(Layer::from_features("others", others)),
                Arc::new(Layer::from_features("polygons", polygons)),
            ],
        }
    }
    /// A fresh topology over the case's layers, with every rule that uses a
    /// line graph.
    fn topology(case: &Case) -> (Topology, [LayerId; 3]) {
        let mut t = Topology::new(case.tolerance).expect("tolerance");
        let [a, b, p] = case.layers.clone().map(|layer| t.add_layer(layer));
        for rule in [
            Rule::LineMustNotHaveDangles,
            Rule::LineMustNotHavePseudoNodes,
            Rule::LineMustNotOverlap,
            Rule::LineMustNotSelfOverlap,
            Rule::LineMustNotSelfIntersect,
            Rule::LineMustNotIntersect,
            Rule::LineMustNotIntersectOrTouchInterior,
        ] {
            t.add_rule(rule, a, None).expect("rule");
        }
        for rule in [
            Rule::LineMustNotOverlapWith,
            Rule::LineMustNotIntersectWith,
            Rule::LineMustNotIntersectOrTouchInteriorWith,
        ] {
            t.add_rule(rule, a, Some(b)).expect("rule");
        }
        t.add_rule(Rule::LineMustNotOverlapPolygonBoundaries, a, Some(p))
            .expect("rule");
        (t, [a, b, p])
    }
    fn report(t: &Topology, token: CancellationToken) -> Result<Vec<String>, SinkError<()>> {
        let mut out = Vec::new();
        let summary = t.validate_into_cancellable(token, |spec, v| {
            out.push(format!("{spec:?} {v:?}"));
            Ok(())
        })?;
        out.push(format!("{summary:?}"));
        Ok(out)
    }
    fn reference_report(case: &Case) -> Vec<String> {
        let token = CancellationToken::default();
        let _old = Old::new(&token);
        report(&topology(case).0, token).expect("report")
    }
    fn text(g: &LineGraph) -> String {
        let parts = g
            .parts
            .iter()
            .map(|p| (p.reference, &p.nodes, p.is_closed))
            .collect::<Vec<_>>();
        format!(
            "{:?}\n{parts:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}",
            g.coords,
            g.incident,
            g.contacts,
            g.sources,
            g.order,
            g.matches()
        )
    }
    fn graph(case: &Case, inputs: usize, old: bool) -> String {
        let (t, [a, b, p]) = topology(case);
        let inputs = match inputs {
            0 => vec![(a, InputMode::Linework)],
            1 => vec![(a, InputMode::Linework), (b, InputMode::Linework)],
            _ => vec![(a, InputMode::Linework), (p, InputMode::PolygonBoundary)],
        };
        let token = CancellationToken::default();
        let _old = old.then(|| Old::new(&token));
        let g = cancel::scope(Some(token), || t.graph_inputs(inputs)).expect("graph");
        assert_eq!(g.reference, old);
        text(&g)
    }
    /// Thread counts and task sizes, forced whatever the input size.
    const SPLITS: [(usize, usize); 4] = [(2, 1), (3, 2), (4, 7), (8, 64)];

    /// Compare graphs of seeded cases with those built the reference way,
    /// split into tasks for even seeds.
    fn compare_graphs(seeds: std::ops::Range<u64>) {
        for seed in seeds {
            let case = case(seed, 4 + seed as usize % 24);
            let inputs = seed as usize % 3;
            let expected = graph(&case, inputs, true);
            let (threads, grain) = SPLITS[seed as usize / 2 % SPLITS.len()];
            let found = if seed % 2 == 1 {
                graph(&case, inputs, false)
            } else {
                Workers::forced(threads, grain, || graph(&case, inputs, false))
            };
            assert_eq!(found, expected, "case {seed}");
        }
    }
    #[test]
    fn graphs_equal_those_built_the_reference_way() {
        compare_graphs(0..700);
    }
    #[test]
    fn graphs_of_more_cases_equal_those_built_the_reference_way() {
        compare_graphs(700..1400);
    }

    /// Compare violations of seeded cases with those found on graphs built
    /// the reference way, split into tasks for even seeds.
    fn compare_violations(seeds: std::ops::Range<u64>) {
        for seed in seeds {
            let case = case(seed + 10_000, 4 + seed as usize % 20);
            let expected = reference_report(&case);
            let (threads, grain) = SPLITS[seed as usize / 2 % SPLITS.len()];
            let run = || report(&topology(&case).0, CancellationToken::default());
            let found = if seed % 2 == 1 {
                run()
            } else {
                Workers::forced(threads, grain, run)
            };
            assert_eq!(found.expect("report"), expected, "case {seed}");
        }
    }
    #[test]
    fn violations_equal_those_found_the_reference_way() {
        compare_violations(0..200);
    }
    #[test]
    fn violations_of_more_cases_equal_those_found_the_reference_way() {
        compare_violations(200..400);
    }

    /// A case large enough to be split by itself: a small case beside long
    /// zigzag lines joined end to end, some of them repeated within
    /// tolerance in the other line layer.
    fn large(seed: u64) -> Case {
        let small = case(seed, 24);
        let mut origin = Coord {
            x: f64::INFINITY,
            y: f64::INFINITY,
        };
        for (_, g) in small.layers[0].features() {
            for c in parts(g).iter().flat_map(|l| l.0.iter()) {
                origin.x = origin.x.min(c.x);
                origin.y = origin.y.min(c.y);
            }
        }
        let spacing = 3. * small.tolerance.max(1.) + 1.;
        let mut ends = None;
        let lines = (0..48_u16)
            .map(|line| {
                let x = origin.x + 40. + f64::from(line) * spacing;
                let mut coords = (0..360_u16)
                    .map(|k| Coord {
                        x: x + f64::from(k % 2) * 0.3,
                        y: origin.y + f64::from(k) * 0.5,
                    })
                    .collect::<Vec<_>>();
                if line % 2 == 1 {
                    coords.reverse();
                    coords.extend(ends);
                }
                ends = coords.first().copied();
                LineString(coords)
            })
            .collect::<Vec<_>>();
        let [a, b, p] = small.layers;
        let mut features = a
            .features()
            .map(|(f, g)| (f, g.clone()))
            .collect::<Vec<_>>();
        features.extend(
            (0..)
                .zip(lines.iter().cloned().map(Geometry::from))
                .map(|(k, g)| (100 + k, g)),
        );
        let mut others = b
            .features()
            .map(|(f, g)| (f, g.clone()))
            .collect::<Vec<_>>();
        let shift = small.tolerance / 2.;
        others.extend(lines.iter().step_by(4).zip(0..).map(|(l, k)| {
            let moved = l.map_coords(|c| Coord {
                x: c.x + shift,
                y: c.y,
            });
            (2000 + k, moved.into())
        }));
        Case {
            tolerance: small.tolerance,
            layers: [
                Arc::new(Layer::from_features("lines", features)),
                Arc::new(Layer::from_features("others", others)),
                p,
            ],
        }
    }

    #[test]
    fn thread_pools_change_neither_graphs_nor_violations() {
        for seed in [3, 5] {
            let case = large(seed);
            let (t, [a, ..]) = topology(&case);
            let segments = t.graph(vec![a]).expect("graph").sources.len();
            assert!(segments >= 1 << 14, "{segments} segments");
            let expected = (graph(&case, 1, true), reference_report(&case));
            for threads in [1, 2, 4, 0] {
                let run = || {
                    let token = CancellationToken::default();
                    hooks::watch(&token, None);
                    let report = report(&topology(&case).0, token.clone());
                    let split = hooks::unwatch(&token).workers > 0;
                    assert_eq!(split, rayon::current_num_threads() > 1, "{threads}");
                    (graph(&case, 1, false), report.expect("report"))
                };
                let found = if threads == 0 {
                    run()
                } else {
                    let pool = rayon::ThreadPoolBuilder::new()
                        .num_threads(threads)
                        .build()
                        .expect("pool");
                    pool.install(run)
                };
                assert!(found == expected, "case {seed} in a pool of {threads}");
            }
        }
    }

    /// Steps that construction and matching pass on a case with near matches.
    const STEPS: [&str; 11] = [
        "parts",
        "index",
        "pairs",
        "crack",
        "cluster",
        "noding",
        "contacts",
        "edges",
        "matches.index",
        "matches.pairs",
        "matches.retain",
    ];
    fn cancellation_case() -> Case {
        let mut case = case(17, 60);
        case.tolerance = 0.3;
        case
    }

    #[test]
    fn cancelling_at_any_step_stops_every_thread_and_keeps_the_topology_usable() {
        let case = cancellation_case();
        let expected = reference_report(&case);
        let (threads, grain) = (4, 16);
        let token = CancellationToken::default();
        hooks::watch(&token, None);
        let (t, _) = topology(&case);
        let found = Workers::forced(threads, grain, || report(&t, token.clone()));
        let visits = hooks::unwatch(&token).visits;
        assert_eq!(found.expect("report"), expected);
        for step in STEPS {
            assert!(
                visits.get(step).is_some_and(|&n| n > 0),
                "{step}: {visits:?}"
            );
        }
        let mut r = Random::new(99);
        for (&step, &count) in &visits {
            for visit in [0, r.below(count), count - 1] {
                let (t, _) = topology(&case);
                let token = CancellationToken::default();
                hooks::watch(&token, Some((step, visit)));
                let result = Workers::forced(threads, grain, || report(&t, token.clone()));
                let record = hooks::unwatch(&token);
                assert!(
                    matches!(result, Err(SinkError::Core(Error::Cancelled))),
                    "{step} {visit}: {result:?}"
                );
                assert_eq!(record.live, 0, "{step} {visit}");
                // Each other thread passes at most one more checkpoint.
                let after = record.visits[step];
                assert!(
                    after > visit && after <= visit + threads,
                    "{step} {visit}: {after}"
                );
                let again = Workers::forced(threads, grain, || {
                    report(&t, CancellationToken::default()).expect("report")
                });
                assert_eq!(again, expected, "{step} {visit}");
            }
        }
    }

    #[test]
    fn cancelling_one_of_two_validations_leaves_the_other_whole() {
        let case = cancellation_case();
        let expected = reference_report(&case);
        for (step, wait) in [("noding", "index"), ("matches.pairs", "matches.index")] {
            let (t, _) = topology(&case);
            let cancelled = CancellationToken::default();
            hooks::watch(&cancelled, Some((step, 0)));
            let (first, second) = std::thread::scope(|scope| {
                let first =
                    scope.spawn(|| Workers::forced(4, 16, || report(&t, cancelled.clone())));
                // Start the second while the first builds what both need.
                while !first.is_finished() && !hooks::record(&cancelled).visits.contains_key(wait) {
                    std::thread::yield_now();
                }
                let second = scope
                    .spawn(|| Workers::forced(4, 16, || report(&t, CancellationToken::default())));
                (first.join().expect("first"), second.join().expect("second"))
            });
            let record = hooks::unwatch(&cancelled);
            assert!(
                matches!(first, Err(SinkError::Core(Error::Cancelled))),
                "{step}: {first:?}"
            );
            assert_eq!(record.live, 0, "{step}");
            assert_eq!(second.expect("report"), expected, "{step}");
        }
    }
}
