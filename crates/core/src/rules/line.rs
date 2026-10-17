use crate::{
    Error, FeatureRef, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    engine::line_graph::LineGraph, merge::merge_edges,
};
use geo::{
    Coord, Geometry, Kernel, MultiLineString, Point,
    kernels::{Orientation, RobustKernel},
};
use std::collections::{BTreeMap, BTreeSet};

type Feature = (LayerId, i64);
struct Overlap {
    a: FeatureRef,
    b: Option<FeatureRef>,
    spans: BTreeMap<usize, Vec<[f64; 2]>>,
    contributions: Vec<([usize; 2], usize, [f64; 2])>,
}
/// Which noded line graph structures are violations.
#[derive(Debug, Clone, Copy)]
pub(super) enum LineCheck {
    /// Part endpoints with one incident use.
    Dangles,
    /// Nodes joining exactly two distinct features at their endpoints.
    PseudoNodes,
    /// Linework shared by distinct features.
    Overlap,
    /// Overlaps and interior crossings between distinct features; with `touch`,
    /// also meetings where one feature is interior.
    Intersect { touch: bool },
}

/// Self-overlaps of each feature, plus true self-crossings when `crossings` is set.
pub(super) fn run_self(
    t: &Topology,
    spec: RuleSpec,
    crossings: bool,
) -> Result<Vec<Violation>, Error> {
    let mut out = Vec::new();
    let layer = t.layer(spec.layer)?;
    let features: Box<dyn Iterator<Item = (i64, &Geometry)> + '_> =
        if let Some((_, fid)) = t.selected_feature() {
            Box::new(layer.feature(layer.ranks()[&fid]).into_iter())
        } else {
            Box::new(layer.features())
        };
    for (fid, geometry) in features {
        crate::cancel::check();
        let mut isolated = Topology::new(t.tolerance())?;
        let layer = isolated.add_layer(crate::Layer::from_features(
            "self",
            [(fid, geometry.clone())],
        ));
        let graph = isolated.graph(vec![layer])?;
        let mut errors = overlaps(&graph, spec, true, None, None);
        if crossings {
            // Original passages retain crossings that tolerance clustering
            // would collapse onto a neighboring bend or source endpoint.
            let mut exact = Topology::new(0.)?;
            let exact_layer = exact.add_layer(crate::Layer::from_features(
                "self",
                [(fid, geometry.clone())],
            ));
            let original = exact.graph(vec![exact_layer])?;
            errors.extend(
                self_crossings(&original, spec, t.tolerance())
                    .into_iter()
                    .filter(|v| {
                        Point::try_from(v.geometry.clone())
                            .is_ok_and(|p| !near_open_endpoint(&original, p.0, t.tolerance()))
                    }),
            );
        }
        for mut error in errors {
            if let Some(origin) = &mut error.origin {
                origin.layer = spec.layer;
            }
            out.push(error);
        }
    }
    Ok(out)
}
/// Whether `p` lies within `tolerance` of an endpoint of an open source part.
fn near_open_endpoint(original: &LineGraph, p: Coord, tolerance: f64) -> bool {
    original.parts.iter().enumerate().any(|(i, part)| {
        let coords = &original.original(i).0;
        !part.closed()
            && [coords[0], *coords.last().expect("source endpoint")]
                .iter()
                .any(|&q| crate::engine::cluster::distance(p, q) <= tolerance)
    })
}
pub(super) fn run(t: &Topology, spec: RuleSpec, check: LineCheck) -> Result<Vec<Violation>, Error> {
    let mut layers = vec![spec.layer];
    if let Some(other) = spec.other {
        layers.push(other);
    }
    let graph = t.graph(layers)?;
    Ok(match check {
        LineCheck::Dangles => dangles(&graph, spec),
        LineCheck::PseudoNodes => pseudos(&graph, spec),
        LineCheck::Overlap => overlaps(&graph, spec, false, t.selected_feature(), None),
        LineCheck::Intersect { touch } => {
            let mut accepted = BTreeSet::new();
            let mut violations = overlaps(
                &graph,
                spec,
                false,
                t.selected_feature(),
                Some(&mut accepted),
            );
            violations.extend(meetings(
                &graph,
                spec,
                touch,
                t.selected_feature(),
                &accepted,
            ));
            violations
        }
    })
}
fn point(
    spec: RuleSpec,
    kind: ViolationKind,
    coordinate: Coord,
    a: FeatureRef,
    b: Option<FeatureRef>,
) -> Violation {
    Violation {
        rule: spec.rule,
        kind,
        geometry: Point(coordinate).into(),
        origin: Some(a),
        destination: b,
    }
}
fn dangles(g: &LineGraph, spec: RuleSpec) -> Vec<Violation> {
    let mut out = Vec::new();
    for part in &g.parts {
        if part.closed() || part.nodes.len() < 2 {
            continue;
        }
        for &node in [part.nodes.first(), part.nodes.last()]
            .into_iter()
            .flatten()
        {
            if g.incident[node].len() == 1 {
                out.push(point(
                    spec,
                    ViolationKind::Dangle,
                    g.coords[node],
                    part.reference,
                    None,
                ));
            }
        }
    }
    out
}
fn pseudos(g: &LineGraph, spec: RuleSpec) -> Vec<Violation> {
    let mut out = Vec::new();
    for (node, uses) in g.incident.iter().enumerate() {
        if uses.len() != 2 {
            continue;
        }
        let a = &g.parts[uses[0].part];
        let b = &g.parts[uses[1].part];
        if a.reference.feature() != b.reference.feature() && a.endpoint(node) && b.endpoint(node) {
            let (a, b) = if a.reference.feature() < b.reference.feature() {
                (a.reference, b.reference)
            } else {
                (b.reference, a.reference)
            };
            out.push(point(
                spec,
                ViolationKind::PseudoNode,
                g.coords[node],
                a,
                Some(b),
            ));
        }
    }
    out
}
fn selected(spec: RuleSpec, a: FeatureRef, b: FeatureRef) -> Option<(FeatureRef, FeatureRef)> {
    if a.feature() == b.feature() {
        return None;
    }
    if let Some(other) = spec.other {
        if a.layer == spec.layer && b.layer == other {
            Some((a, b))
        } else if b.layer == spec.layer && a.layer == other {
            Some((b, a))
        } else {
            None
        }
    } else if a.feature() < b.feature() {
        Some((a, b))
    } else {
        Some((b, a))
    }
}
fn overlaps(
    g: &LineGraph,
    spec: RuleSpec,
    self_rule: bool,
    selected_feature: Option<Feature>,
    mut accepted: Option<&mut BTreeSet<[usize; 2]>>,
) -> Vec<Violation> {
    use crate::engine::matching;
    let mut groups: BTreeMap<(Feature, Option<Feature>), Overlap> = BTreeMap::new();
    // Choose an immutable representative by geometry, independent of FIDs,
    // input order and traversal direction. Unequal segmentation remains local.
    for matched in g.selected_matches(selected_feature) {
        crate::cancel::check();
        #[cfg(test)]
        crate::engine::line_graph::MATCHING_WORK.with(|n| n.set(n.get() + 1));
        let [x, y] = matched.sources;
        let (xp, yp) = (g.segment(x).part, g.segment(y).part);
        let (xr, yr) = (g.parts[xp].reference, g.parts[yp].reference);
        let (a, b) = if self_rule {
            // Self rules run on one isolated feature, so both sides share it.
            let mut a = xr;
            a.aggregate(yr);
            (a, None)
        } else {
            let Some((a, b)) = selected(spec, xr, yr) else {
                continue;
            };
            if selected_feature.is_some_and(|key| a.feature() != key) {
                continue;
            }
            (a, Some(b))
        };
        let (ac, bc) = (&g.original(xp).0, &g.original(yp).0);
        let (ar, br) = (g.canonical_reversed(xp), g.canonical_reversed(yp));
        let cmp = (0..ac.len().min(bc.len()))
            .map(|i| {
                let a = ac[if ar { ac.len() - i - 1 } else { i }];
                let b = bc[if br { bc.len() - i - 1 } else { i }];
                a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y))
            })
            .find(|o| !o.is_eq())
            .unwrap_or_else(|| ac.len().cmp(&bc.len()));
        let side = usize::from(cmp.is_gt());
        let group = groups
            .entry((a.feature(), b.map(FeatureRef::feature)))
            .or_insert(Overlap {
                a,
                b,
                spans: BTreeMap::new(),
                contributions: Vec::new(),
            });
        group.a.aggregate(a);
        if let (Some(old), Some(b)) = (&mut group.b, b) {
            old.aggregate(b);
        }
        if accepted.is_some() {
            group
                .contributions
                .push((matched.sources, matched.sources[side], matched.spans[side]));
        }
        group
            .spans
            .entry(matched.sources[side])
            .or_default()
            .push(matched.spans[side]);
    }
    groups
        .into_values()
        .filter_map(|group| {
            let mut coords = Vec::new();
            let mut nodes = BTreeMap::new();
            let mut edges = Vec::new();
            let mut contributions = Vec::new();
            for (source, spans) in group.spans {
                for span in matching::union(spans) {
                    let mut pair = [0; 2];
                    for (i, t) in span.into_iter().enumerate() {
                        let p = matching::point(g.source(source), t);
                        let key = (
                            crate::engine::cluster::bits(p.x),
                            crate::engine::cluster::bits(p.y),
                        );
                        pair[i] = *nodes.entry(key).or_insert_with(|| {
                            let id = coords.len();
                            coords.push(p);
                            id
                        });
                    }
                    if pair[0] != pair[1] {
                        edges.push(pair);
                        if accepted.is_some() {
                            contributions.push((source, span, pair));
                        }
                    }
                }
            }
            let mut lines = merge_edges(&coords, &edges);
            use geo::Length;
            lines.retain(|line| geo::Euclidean.length(line) > g.tolerance);
            if lines.is_empty() {
                return None;
            }
            if let Some(accepted) = accepted.as_deref_mut() {
                let key = |p: Coord| {
                    (
                        crate::engine::cluster::bits(p.x),
                        crate::engine::cluster::bits(p.y),
                    )
                };
                let kept = lines
                    .iter()
                    .flat_map(|line| line.0.windows(2))
                    .map(|pair| {
                        let a = nodes[&key(pair[0])];
                        let b = nodes[&key(pair[1])];
                        [a.min(b), a.max(b)]
                    })
                    .collect::<BTreeSet<_>>();
                let mut spans = BTreeMap::<_, Vec<_>>::new();
                for (source, span, pair) in contributions {
                    if kept.contains(&[pair[0].min(pair[1]), pair[0].max(pair[1])]) {
                        spans.entry(source).or_default().push(span);
                    }
                }
                for (sources, representative, span) in group.contributions {
                    if spans
                        .get(&representative)
                        .is_some_and(|kept| kept.iter().any(|s| s[0] <= span[0] && s[1] >= span[1]))
                    {
                        accepted.insert(sources);
                    }
                }
            }
            let geometry = if lines.len() == 1 {
                Geometry::LineString(lines.remove(0))
            } else {
                MultiLineString(lines).into()
            };
            Some(Violation {
                rule: spec.rule,
                kind: if self_rule {
                    ViolationKind::SelfOverlap
                } else {
                    ViolationKind::Overlap
                },
                geometry,
                origin: Some(group.a),
                destination: group.b,
            })
        })
        .collect()
}
fn meetings(
    g: &LineGraph,
    spec: RuleSpec,
    touch: bool,
    selected_feature: Option<Feature>,
    accepted: &BTreeSet<[usize; 2]>,
) -> Vec<Violation> {
    let mut out = Vec::new();
    let accepted_matches = accepted
        .iter()
        .map(|sources| {
            &g.matches()[g
                .matches()
                .binary_search_by_key(sources, |m| m.sources)
                .expect("accepted source match")]
        })
        .collect::<Vec<_>>();
    let accepted_index =
        crate::engine::index::build(accepted_matches.iter().enumerate().map(|(i, m)| {
            let line = g.source(m.sources[0]);
            (
                i,
                geo::Rect::new(
                    crate::engine::matching::point(line, m.spans[0][0]),
                    crate::engine::matching::point(line, m.spans[0][1]),
                ),
            )
        }));
    // Contacts are keyed by their own coordinate. Visit the same nodes in both paths,
    // since that node need not belong to either noded part.
    let nodes: Box<dyn Iterator<Item = usize> + '_> = match selected_feature {
        Some(key) => Box::new(g.feature_contacts(key).iter().copied()),
        None => Box::new(g.contacts.keys().copied()),
    };
    for node in nodes {
        crate::cancel::check();
        let mut pairs = BTreeMap::<_, (FeatureRef, FeatureRef, bool)>::new();
        for contact in g.contacts.get(&node).into_iter().flatten() {
            let [a, b] = contact.parts.map(|part| g.parts[part].reference);
            let Some((origin, destination)) = selected(spec, a, b) else {
                continue;
            };
            if selected_feature.is_some_and(|key| origin.feature() != key) {
                continue;
            }
            // Suppression requires this exact original correspondence to have
            // contributed to retained overlap geometry, after the length test.
            let covered = accepted_index
                .locate_in_envelope_intersecting(&crate::engine::index::at(
                    contact.points[0],
                    g.tolerance,
                ))
                .any(|item| {
                    crate::cancel::check();
                    let m = accepted_matches[item.data];
                    [0, 1].iter().all(|&side| {
                        let source = m.sources[side];
                        // A part of the same feature with a vertex at the contact
                        // continues the passage, as another segment of one part does.
                        let feature = g.parts[g.segment(source).part].reference.feature();
                        let Some(contact_side) = contact
                            .parts
                            .iter()
                            .position(|&p| g.parts[p].reference.feature() == feature)
                        else {
                            return false;
                        };
                        let point = contact.points[contact_side];
                        let contact_source = contact.sources[contact_side];
                        if source != contact_source {
                            let a = g.source(source);
                            let b = g.source(contact_source);
                            // Adjacent passages can share a retained overlap endpoint.
                            // Clustering or nearby detached runs cannot supply this link.
                            let shared = [a.start, a.end].contains(&point)
                                && [b.start, b.end].contains(&point);
                            if !shared && !rounded_vertex(g, contact, contact_side, source) {
                                return false;
                            }
                        }

                        let line = g.source(source);
                        let (parameter, projected) =
                            crate::engine::line_graph::projection(line, point);
                        // A cracked endpoint often defines the span boundary it is
                        // tested against. Its projection differs from that boundary by
                        // rounding, and by up to its offset from the line when the
                        // segments are not parallel, so allow one tolerance along it.
                        let slack =
                            g.tolerance / crate::engine::cluster::distance(line.start, line.end);
                        parameter >= m.spans[side][0] - slack
                            && parameter <= m.spans[side][1] + slack
                            && crate::engine::cluster::distance(projected, point) <= g.tolerance
                    })
                });
            if covered {
                continue;
            }
            let crossing = contact.interior.iter().all(|&interior| interior);
            if !crossing && !touch {
                continue;
            }
            let entry = pairs
                .entry((origin.feature(), destination.feature()))
                .or_insert((origin, destination, false));
            entry.0.aggregate(origin);
            entry.1.aggregate(destination);
            entry.2 |= crossing;
        }
        for (origin, destination, crossing) in pairs.into_values() {
            out.push(point(
                spec,
                if crossing {
                    ViolationKind::Crossing
                } else {
                    ViolationKind::TouchInterior
                },
                g.coords[node],
                origin,
                Some(destination),
            ));
        }
    }
    out
}
/// Whether a contact on the source next to `source` in the same part is their shared
/// vertex, displaced only by rounding of input coordinates. The error is measured
/// backward: a projected endpoint must land on the vertex, and an intersection is the
/// vertex when the vertex lies on the other segment. A shallow adjacent segment can
/// move its intersection far from a vertex that is off the line by one rounding step.
fn rounded_vertex(
    g: &LineGraph,
    contact: &crate::engine::line_graph::Contact,
    side: usize,
    source: usize,
) -> bool {
    use crate::engine::cluster::distance;
    let [s, c] = [source, contact.sources[side]].map(|i| g.segment(i));
    if s.part != c.part {
        return false;
    }
    let coords = &g.original(s.part).0;
    let vertex = if s.start.abs_diff(c.start) == 1 {
        coords[s.start.max(c.start)]
    } else if g.parts[s.part].closed()
        && s.start.min(c.start) == 0
        && s.start.max(c.start) == coords.len() - 2
    {
        coords[0]
    } else {
        return false;
    };
    let point = contact.points[side];
    let other = g.source(contact.sources[1 - side]);
    let own = g.source(contact.sources[side]);
    let epsilon = crate::engine::line_graph::rounding(&[
        point,
        contact.points[1 - side],
        own.start,
        own.end,
        other.start,
        other.end,
    ])
    .min(g.tolerance);
    let offset = distance(point, vertex);
    offset <= epsilon
        || contact.points[0] == contact.points[1]
            && offset <= g.tolerance
            && distance(
                crate::engine::line_graph::projection(other, vertex).1,
                vertex,
            ) <= epsilon
}

fn crosses(origin: Coord, a: [Coord; 2], b: [Coord; 2]) -> bool {
    let mut rays = [(a[0], 0), (a[1], 0), (b[0], 1), (b[1], 1)];
    // Equal rays describe a tangency or an overlap, not a true crossing.
    for i in 0..4 {
        for j in i + 1..4 {
            let p = rays[i].0;
            let q = rays[j].0;
            if RobustKernel::orient2d(origin, p, q) == Orientation::Collinear
                && (p.x - origin.x) * (q.x - origin.x) + (p.y - origin.y) * (q.y - origin.y) > 0.
            {
                return false;
            }
        }
    }
    rays.sort_by(|a, b| {
        (a.0.y - origin.y)
            .atan2(a.0.x - origin.x)
            .total_cmp(&(b.0.y - origin.y).atan2(b.0.x - origin.x))
    });
    (0..4).all(|i| rays[i].1 != rays[(i + 1) % 4].1)
}
fn self_crossings(g: &LineGraph, spec: RuleSpec, tolerance: f64) -> Vec<Violation> {
    type Passage = (FeatureRef, [usize; 2]);
    let mut visits: BTreeMap<(usize, Feature), Vec<Passage>> = BTreeMap::new();
    for (part_id, part) in g.parts.iter().enumerate() {
        if crate::engine::line_graph::collapsed(g.original(part_id), tolerance) {
            continue;
        }
        for triple in part.nodes.windows(3) {
            visits
                .entry((triple[1], part.reference.feature()))
                .or_default()
                .push((part.reference, [triple[0], triple[2]]));
        }
        if part.closed() && part.nodes.len() > 2 {
            visits
                .entry((part.nodes[0], part.reference.feature()))
                .or_default()
                .push((
                    part.reference,
                    [part.nodes[part.nodes.len() - 2], part.nodes[1]],
                ));
        }
    }
    let mut out = Vec::new();
    for ((node, _), passes) in visits {
        let mut origin: Option<FeatureRef> = None;
        for i in 0..passes.len() {
            for j in i + 1..passes.len() {
                let a = passes[i];
                let b = passes[j];
                if crosses(
                    g.coords[node],
                    a.1.map(|n| g.coords[n]),
                    b.1.map(|n| g.coords[n]),
                ) {
                    let reference = origin.get_or_insert(a.0);
                    reference.aggregate(a.0);
                    reference.aggregate(b.0);
                }
            }
        }
        if let Some(origin) = origin {
            out.push(point(
                spec,
                ViolationKind::SelfCrossing,
                g.coords[node],
                origin,
                None,
            ));
        }
    }
    out
}
