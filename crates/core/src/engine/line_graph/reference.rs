//! The construction the faster one replaced, kept to check that they give the
//! same graphs.
use super::*;
use crate::engine::cluster::{Cluster, bits};

fn add_cut(cuts: &mut HashMap<usize, Vec<Coord>>, source: usize, line: Line, point: Coord) {
    if point != line.start && point != line.end {
        cuts.entry(source).or_default().push(point);
    }
}
impl LineGraph {
    /// The graph [`LineGraph::build`] makes, built on one thread with an
    /// R-tree, hashed end counts and hashed clusters.
    pub(super) fn reference(t: &Topology, layers: &[(LayerId, InputMode)]) -> Result<Self, Error> {
        let tolerance = t.tolerance();
        let mut parts = Vec::new();
        let mut segments = Vec::new();
        // Open part ends with their segments.
        let mut endpoints = Vec::new();
        let mut coordinates = Vec::new();
        for &(layer, mode) in layers {
            let input = t.parts(layer)?;
            let lines: Vec<(crate::model::PartRef, &Shared<LineString>)> = match mode {
                InputMode::Linework => input
                    .lines
                    .iter()
                    .map(|p| (p.reference, &p.geometry))
                    .collect(),
                InputMode::PolygonBoundary => crate::engine::cross::boundaries(input)
                    .iter()
                    .map(|p| (p.reference, &p.geometry))
                    .collect(),
            };
            for (reference, geometry) in lines {
                if mode == InputMode::Linework && collapsed(geometry, tolerance) {
                    continue;
                }
                let coords = &geometry.0;
                coordinates.extend_from_slice(coords);
                let closed = coords.first() == coords.last();
                if !closed {
                    let last = segments.len() + coords.len() - 2;
                    endpoints.extend([
                        (coords[0], segments.len()),
                        (coords[coords.len() - 1], last),
                    ]);
                }
                let part = parts.len();
                segments.extend((0..coords.len() - 1).map(|start| Segment { start, part }));
                parts.push(GraphPart {
                    reference: reference.at(layer),
                    nodes: Vec::new(),
                    is_closed: closed,
                    original: geometry.clone(),
                });
            }
        }
        let tree = index::build(
            segments
                .iter()
                .enumerate()
                .map(|(i, s)| (i, s.line(&parts).bounding_rect())),
        );
        let mut split = HashMap::new();
        let mut endpoint_counts = HashMap::<_, usize>::new();
        let mut feature_ends = Vec::new();
        for &(point, source) in &endpoints {
            let feature = parts[segments[source].part].reference.feature();
            *endpoint_counts
                .entry((feature, bits(point.x), bits(point.y)))
                .or_default() += 1;
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
            let [l0, l1] = sources.map(|source| segments[source].line(&parts));
            let interior = [0, 1].map(|side| {
                let p = &parts[[a, b][side]];
                let feature = p.reference.feature();
                let point = points[side];
                let count = |end: Coord| {
                    endpoint_counts
                        .get(&(feature, bits(end.x), bits(end.y)))
                        .copied()
                };
                let nearest = || {
                    let own = [p.original.0[0], *p.original.0.last().expect("source")]
                        .into_iter()
                        .filter(|&end| !p.closed() && distance(point, end) <= tolerance);
                    let base = rounding(&[point, l0.start, l0.end, l1.start, l1.end]);
                    let window = 2. * base.min(tolerance);
                    let others = ends_near(&feature_ends, feature, point, window).filter(|&end| {
                        distance(point, end) <= base.max(rounding(&[end])).min(tolerance)
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
            if interior[0] || interior[1] {
                let contact = Contact {
                    parts: [a, b],
                    interior,
                    points,
                    sources,
                };
                original_contacts.push((coordinate, contact));
            }
        };
        let near = |vertex: Coord, segment: Line| {
            let (_, projected) = projection(segment, vertex);
            (tolerance > 0.
                && distance(vertex, projected)
                    <= rounding(&[vertex, segment.start, segment.end]).min(tolerance))
            .then_some(projected)
        };
        let vertex = |s: &Segment| {
            let part = &parts[s.part];
            (s.start > 0 || part.closed()).then(|| part.original.0[s.start])
        };
        let reach = rounding(&coordinates).min(tolerance);
        for (i, s) in segments.iter().enumerate() {
            let line = s.line(&parts);
            let bounds = index::expanded(line.bounding_rect(), reach);
            for item in tree.locate_in_envelope_intersecting(&bounds) {
                let j = item.data;
                if j <= i {
                    continue;
                }
                let other = segments[j].line(&parts);
                match line_intersection(line, other) {
                    Some(LineIntersection::SinglePoint {
                        intersection,
                        is_proper,
                    }) => {
                        let point = if is_proper {
                            proper_intersection(line, other).unwrap_or(intersection)
                        } else {
                            intersection
                        };
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
                            .min_by(|a, b| distance(a.0, point).total_cmp(&distance(b.0, point)));
                        match beside {
                            Some((p, points)) => contact(i, j, points, p),
                            None => contact(i, j, [point, point], point),
                        }
                        add_cut(&mut split, i, line, point);
                        add_cut(&mut split, j, other, point);
                    }
                    Some(LineIntersection::Collinear { intersection }) => {
                        for p in [intersection.start, intersection.end] {
                            contact(i, j, [p, p], p);
                            add_cut(&mut split, i, line, p);
                            add_cut(&mut split, j, other, p);
                        }
                    }
                    None => {}
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
        let mut ties = HashMap::<(usize, bool), Vec<Coord>>::new();
        if tolerance > 0. {
            for &(point, source) in &endpoints {
                let mut candidates = tree
                    .locate_in_envelope_intersecting(&index::at(point, tolerance))
                    .map(|i| i.data)
                    .collect::<Vec<_>>();
                candidates.sort_unstable();
                let part = segments[source].part;
                let (mut own, mut others) = (false, Vec::new());
                for i in candidates {
                    let line = segments[i].line(&parts);
                    let (_, projected) = projection(line, point);
                    if distance(point, projected) <= tolerance {
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
        let order = tree.iter().map(|i| u32::try_from(i.data).ok()).collect();
        let mut cluster = Cluster::new(tolerance);
        let ends = endpoints.into_iter().map(|(point, _)| point).collect();
        let mut points = coordinates;
        points.extend(split.values().flatten().copied());
        for mut points in [ends, cracked, points] {
            points.sort_by(coordinate_order);
            points.dedup();
            for point in points {
                cluster.insert(point);
            }
        }
        let mut raw_edges = Vec::new();
        let mut path: Vec<Visit> = Vec::new();
        let mut trims = Vec::new();
        for (source, s) in segments.iter().enumerate() {
            let line = s.line(&parts);
            let mut cuts = split.remove(&source).unwrap_or_default();
            cuts.extend([line.start, line.end]);
            cuts.sort_by(|a, b| parameter(line, *a).total_cmp(&parameter(line, *b)));
            cuts.dedup();
            let mut previous = None;
            for point in cuts {
                let node = cluster.insert(point);
                let at = (point, s.start);
                if previous == Some(node) {
                    if let Some(visit) = path.last_mut() {
                        visit.last = at;
                    }
                    continue;
                }
                let edge = previous.map(|p: usize| {
                    let usage = EdgeUse {
                        part: s.part,
                        source,
                    };
                    ([p.min(node), p.max(node)], usage)
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
            let bounds = if part.is_closed || tolerance == 0. {
                [0, path.len() - 1]
            } else {
                kept_visits(&path, &part.original.0, tolerance)
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
            let ties: HashMap<_, Vec<usize>> = ties
                .into_iter()
                .map(|(end, points)| {
                    let nodes = points.iter().map(|&p| cluster.get(p).expect("crack point"));
                    (end, nodes.collect())
                })
                .collect();
            let mut list = Vec::new();
            let mut spans = Vec::new();
            for part in &parts {
                spans.push(list.len()..list.len() + part.nodes.len());
                list.extend(&part.nodes);
            }
            trim_returns(&parts, &list, &mut spans, &mut raw_edges, trims, &ties);
            for (part, span) in parts.iter_mut().zip(spans) {
                part.nodes = list[span].to_vec();
            }
        }
        let mut by_node = BTreeMap::<_, Vec<_>>::new();
        for (coordinate, contact) in original_contacts {
            let node = cluster.insert(coordinate);
            by_node.entry(node).or_default().push(contact);
        }
        let mut contacts = Contacts {
            nodes: Vec::new(),
            offsets: vec![0],
            values: Vec::new(),
        };
        for (node, list) in by_node {
            contacts.nodes.push(node);
            contacts.values.extend(list);
            contacts.offsets.push(contacts.values.len());
        }
        let coords = cluster.finish();
        // Each node lists the parts of its edges in the order of their nodes,
        // parts and sources.
        raw_edges.sort_unstable_by_key(|e| (e.0, e.1.part, e.1.source));
        let mut lists = vec![Vec::new(); coords.len()];
        for (nodes, usage) in raw_edges {
            for node in nodes {
                lists[node].push(Incident { part: usage.part });
            }
        }
        let offsets = std::iter::once(0)
            .chain(lists.iter().scan(0, |n, list| {
                *n += list.len();
                Some(*n)
            }))
            .collect();
        let incident = Incidences {
            offsets,
            values: lists.into_iter().flatten().collect(),
        };
        let graph = Self {
            matches: Cache::new(),
            match_pairs: Cache::new(),
            match_features: Cache::new(),
            canonical_directions: Cache::new(),
            tolerance,
            contact_nodes: Cache::new(),
            coords,
            sources: segments,
            parts,
            incident,
            contacts,
            order,
        };
        graph.matches.get_or_init(|| graph.reference_matches());
        Ok(graph)
    }
    /// The matches [`LineGraph::matches`] finds, found on one thread with an
    /// R-tree.
    fn reference_matches(&self) -> Vec<Matched> {
        let tree =
            index::build((0..self.sources.len()).map(|i| (i, self.source(i).bounding_rect())));
        let (mut result, evidence): (Vec<_>, Vec<_>) =
            index::pairs(&tree, &tree, self.tolerance, true)
                .filter_map(|(a, b)| {
                    let support = crate::engine::matching::evidence(
                        self.source(a),
                        self.source(b),
                        self.tolerance,
                    )?;
                    let matched = Matched {
                        sources: [a, b],
                        spans: support.spans,
                        overlap: true,
                    };
                    Some((matched, support))
                })
                .unzip();
        if self.tolerance > 0. && !result.is_empty() {
            self.retain_sustained_matches(&tree, &Workers::new(0), &mut result, &evidence);
        }
        result.sort_unstable_by_key(|m| m.sources);
        result
    }
}

mod tests {
    use super::*;
    use crate::{CancellationToken, Layer, Rule, SinkError, cancel::hooks};
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
    /// Everything of a graph that rules read, other than what they derive
    /// from it.
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
    /// Texts of the graphs that rules on a case use: of its lines with the
    /// other lines or with the polygon boundaries for `inputs` 1 and 2; for 0
    /// of its lines, then of each line feature alone at the case's tolerance
    /// and at zero.
    fn graphs(case: &Case, inputs: usize, reference: bool) -> Vec<String> {
        let build = |t: &Topology, layers: Vec<(LayerId, InputMode)>| {
            if reference {
                text(&LineGraph::reference(t, &layers).expect("graph"))
            } else {
                text(&t.graph_inputs(layers).expect("graph"))
            }
        };
        let (t, [a, b, p]) = topology(case);
        let layers = match inputs {
            0 => vec![(a, InputMode::Linework)],
            1 => vec![(a, InputMode::Linework), (b, InputMode::Linework)],
            _ => vec![(a, InputMode::Linework), (p, InputMode::PolygonBoundary)],
        };
        let mut out = vec![build(&t, layers)];
        if inputs > 0 {
            return out;
        }
        for (fid, geometry) in case.layers[0].features() {
            for tolerance in [case.tolerance, 0.] {
                let mut alone = Topology::new(tolerance).expect("tolerance");
                let layer =
                    alone.add_layer(Layer::from_features("self", [(fid, geometry.clone())]));
                out.push(build(&alone, vec![(layer, InputMode::Linework)]));
            }
        }
        out
    }
    fn assert_same(found: &[String], expected: &[String], case: &str) {
        assert_eq!(found.len(), expected.len(), "{case}");
        for (k, (found, expected)) in found.iter().zip(expected).enumerate() {
            assert_eq!(found, expected, "{case}, graph {k}");
        }
    }
    /// Thread counts and task sizes, forced whatever the input size.
    const SPLITS: [(usize, usize); 4] = [(2, 1), (3, 2), (4, 7), (8, 64)];

    /// Compare graphs of seeded cases with those built the reference way,
    /// split into tasks for even seeds. Rules read nothing else of their
    /// inputs, so equal graphs give equal violations.
    fn compare_graphs(seeds: std::ops::Range<u64>) {
        for seed in seeds {
            let case = case(seed, 4 + seed as usize % 24);
            let inputs = seed as usize % 3;
            let expected = graphs(&case, inputs, true);
            let (threads, grain) = SPLITS[seed as usize / 2 % SPLITS.len()];
            let found = if seed % 2 == 1 {
                graphs(&case, inputs, false)
            } else {
                Workers::forced(threads, grain, || graphs(&case, inputs, false))
            };
            assert_same(&found, &expected, &format!("case {seed}"));
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

    /// Compare violations of seeded cases split into tasks with those found
    /// on the calling thread.
    fn compare_violations(seeds: std::ops::Range<u64>) {
        for seed in seeds {
            let case = case(seed + 10_000, 4 + seed as usize % 20);
            let run = || report(&topology(&case).0, CancellationToken::default());
            let expected = run().expect("report");
            let (threads, grain) = SPLITS[seed as usize % SPLITS.len()];
            let found = Workers::forced(threads, grain, run).expect("report");
            assert_eq!(found, expected, "case {seed}");
        }
    }
    #[test]
    fn splitting_into_tasks_changes_no_violations() {
        compare_violations(0..200);
    }
    #[test]
    fn splitting_more_cases_into_tasks_changes_no_violations() {
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
            let pool = |threads| {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .expect("pool")
            };
            let run = || report(&topology(&case).0, CancellationToken::default());
            let expected = (
                graphs(&case, 1, true),
                pool(1).install(run).expect("report"),
            );
            for threads in [2, 4, 0] {
                let run = || {
                    let token = hooks::watched(None);
                    let report = report(&topology(&case).0, token.clone());
                    assert!(hooks::record(&token).workers > 0, "{threads}");
                    (graphs(&case, 1, false), report.expect("report"))
                };
                let found = if threads == 0 {
                    run()
                } else {
                    pool(threads).install(run)
                };
                let name = format!("case {seed} in a pool of {threads}");
                assert_same(&found.0, &expected.0, &name);
                assert!(found.1 == expected.1, "{name}");
            }
        }
    }
    #[test]
    fn small_inputs_are_handled_on_the_calling_thread() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .expect("pool");
        for seed in 0..20 {
            let case = case(seed, 28);
            let token = hooks::watched(None);
            pool.install(|| {
                cancel::scope(Some(token.clone()), || {
                    for inputs in 0..3 {
                        graphs(&case, inputs, false);
                    }
                })
            });
            assert_eq!(hooks::record(&token).workers, 0, "case {seed}");
        }
    }

    /// A case that passes every checkpoint of construction and matching.
    fn cancellation_case() -> Case {
        case(2, 40)
    }
    /// The checkpoints it passes.
    const STEPS: &str = "\
        aggregate aggregate.gather aggregate.join aggregate.merge aggregate.sort cluster \
        cluster.batch cluster.ends cluster.points cluster.sort collapsed connected.components connected.join connected.sort connected.spans contacts \
        contacts.features contacts.nodes contacts.sort contacts.values crack cuts dangles \
        directions edges edges.count edges.cursor edges.group edges.incident edges.incident.init \
        edges.offsets edges.sort ends ends.init ends.sort fill gather.contacts gather.edges index \
        index.arrange matches.features matches.index matches.pairs matches.retain matches.sort \
        noding order pairs pairs.join parts pseudos retain.components retain.confined \
        retain.found retain.groups retain.ids retain.junctions retain.junctions.sort \
        retain.meetings retain.meetings.sort retain.positions retain.runs retain.spans \
        retain.spans.join retain.spans.sort retain.support retain.support.spans returns.chord \
        returns.first returns.last returns.near returns.segments sort.buckets sort.classes \
        sort.copy sort.moves sort.samples sort.serial support.each support.evidence \
        support.groups support.ids support.lengths support.order support.participants \
        support.passages support.roles support.roles.assign support.runs support.runs.join \
        support.runs.sort sustained.component sustained.lengths sustained.lines sustained.merge \
        sustained.pending sustained.restore sustained.sort sustained.sources sustained.widths \
        sweep.cells sweep.cells.sort sweep.earlier ties trim.dropped trim.edges trim.ends \
        trim.own trim.parts trim.removed violations.sort zones.explained zones.meetings \
        zones.relevant";

    #[test]
    fn cancelling_at_any_step_stops_every_thread_and_keeps_the_topology_usable() {
        let case = cancellation_case();
        let (threads, grain) = (4, 16);
        let run = |t: &Topology, token| Workers::forced(threads, grain, || report(t, token));
        let expected = report(&topology(&case).0, CancellationToken::default()).expect("report");
        let token = hooks::watched(None);
        let found = run(&topology(&case).0, token.clone());
        assert_eq!(found.expect("report"), expected);
        let visits = hooks::record(&token).visits;
        let steps = visits.keys().copied().collect::<Vec<_>>();
        assert_eq!(steps, STEPS.split_whitespace().collect::<Vec<_>>());
        let mut r = Random::new(99);
        for (&step, &count) in &visits {
            for visit in [0, r.below(count), count - 1] {
                let (t, _) = topology(&case);
                let token = hooks::watched(Some((step, visit)));
                let result = run(&t, token.clone());
                let record = hooks::record(&token);
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
                let again = run(&t, CancellationToken::default()).expect("report");
                assert_eq!(again, expected, "{step} {visit}");
            }
        }
    }

    #[test]
    fn cancelling_one_of_two_validations_leaves_the_other_whole() {
        let case = cancellation_case();
        let expected = report(&topology(&case).0, CancellationToken::default()).expect("report");
        for (step, wait) in [("noding", "index"), ("matches.pairs", "matches.index")] {
            let (t, _) = topology(&case);
            let cancelled = hooks::watched(Some((step, 0)));
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
            assert!(
                matches!(first, Err(SinkError::Core(Error::Cancelled))),
                "{step}: {first:?}"
            );
            assert_eq!(hooks::record(&cancelled).live, 0, "{step}");
            assert_eq!(second.expect("report"), expected, "{step}");
        }
    }

    /// Lines of one segment on a grid, some meeting at their ends and some
    /// overlapping.
    fn tiny_lines(count: usize) -> Topology {
        let features = (0..count)
            .map(|k| {
                let (x, y) = ((k % 200) as f64, (k / 200) as f64);
                let (dx, dy) = [(1., 0.), (0., 0.5), (0.5, 0.5), (2., 0.)][k % 4];
                let line = LineString(vec![
                    Coord { x, y },
                    Coord {
                        x: x + dx,
                        y: y + dy,
                    },
                ]);
                (k as i64, Geometry::from(line))
            })
            .collect::<Vec<_>>();
        let mut t = Topology::new(0.001).expect("tolerance");
        let a = t.add_layer(Layer::from_features("tiny", features));
        t.add_rule(Rule::LineMustNotHaveDangles, a, None)
            .expect("rule");
        t.add_rule(Rule::LineMustNotOverlap, a, None).expect("rule");
        t
    }
    #[test]
    fn many_tiny_parts_are_cancelled_in_every_step() {
        let count = 20_000;
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .expect("pool");
        let token = hooks::watched(None);
        pool.install(|| report(&tiny_lines(count), token.clone()))
            .expect("report");
        let record = hooks::record(&token);
        assert!(record.workers > 0);
        // Loops over all parts, ends, points, segments and edges check as they go.
        for step in ["parts", "directions"] {
            assert!(record.visits[step] >= count, "{step}: {:?}", record.visits);
        }
        for step in [
            "ends",
            "cluster.points",
            "pairs",
            "noding",
            "edges",
            "edges.count",
        ] {
            let least = count / crate::cancel::CHECK;
            assert!(record.visits[step] >= least, "{step}: {:?}", record.visits);
        }
        for (&step, &visits) in &record.visits {
            let t = tiny_lines(count);
            let token = hooks::watched(Some((step, visits / 2)));
            let result = pool.install(|| report(&t, token.clone()));
            assert!(
                matches!(result, Err(SinkError::Core(Error::Cancelled))),
                "{step}: {result:?}"
            );
            assert_eq!(hooks::record(&token).live, 0, "{step}");
        }
    }
}
