use crate::{
    Error, FeatureRef, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    engine::line_graph::LineGraph, merge::merge_edges,
};
use geo::{
    Coord, Geometry, Kernel, LineString, MultiLineString, Point,
    kernels::{Orientation, RobustKernel},
};
use std::collections::BTreeMap;

type Feature = (LayerId, i64);
struct Overlap {
    a: FeatureRef,
    b: Option<FeatureRef>,
    edges: Vec<usize>,
}
#[derive(Clone, Copy)]
struct Meeting {
    reference: FeatureRef,
    interior: bool,
    endpoints: usize,
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
    /// Linework traversed repeatedly by one feature.
    SelfOverlap,
    /// Self-overlaps and true self-crossings.
    SelfIntersect,
    /// Overlaps and interior crossings between distinct features; with `touch`,
    /// also meetings where one feature is interior.
    Intersect { touch: bool },
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
        LineCheck::Overlap => overlaps(&graph, spec, false),
        LineCheck::SelfOverlap => overlaps(&graph, spec, true),
        LineCheck::SelfIntersect => {
            let mut violations = overlaps(&graph, spec, true);
            violations.extend(self_crossings(&graph, spec));
            violations
        }
        LineCheck::Intersect { touch } => {
            let mut violations = overlaps(&graph, spec, false);
            violations.extend(meetings(&graph, spec, touch));
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
fn overlaps(g: &LineGraph, spec: RuleSpec, self_rule: bool) -> Vec<Violation> {
    let mut groups: BTreeMap<(Feature, Option<Feature>), Overlap> = BTreeMap::new();
    for (edge_id, edge) in g.edges.iter().enumerate() {
        let mut features: BTreeMap<Feature, (FeatureRef, usize)> = BTreeMap::new();
        for usage in &edge.uses {
            let reference = g.parts[usage.part].reference;
            let entry = features
                .entry(reference.feature())
                .or_insert((reference, 0));
            entry.0.aggregate(reference);
            entry.1 += 1;
        }
        if self_rule {
            for (key, (reference, count)) in features {
                if count > 1 {
                    let group = groups.entry((key, None)).or_insert(Overlap {
                        a: reference,
                        b: None,
                        edges: Vec::new(),
                    });
                    group.a.aggregate(reference);
                    group.edges.push(edge_id);
                }
            }
        } else {
            let features = features.into_values().map(|(r, _)| r).collect::<Vec<_>>();
            for i in 0..features.len() {
                for j in i + 1..features.len() {
                    let Some((a, b)) = selected(spec, features[i], features[j]) else {
                        continue;
                    };
                    let group = groups
                        .entry((a.feature(), Some(b.feature())))
                        .or_insert(Overlap {
                            a,
                            b: Some(b),
                            edges: Vec::new(),
                        });
                    group.a.aggregate(a);
                    if let Some(old) = &mut group.b {
                        old.aggregate(b);
                    }
                    group.edges.push(edge_id);
                }
            }
        }
    }
    groups
        .into_values()
        .map(|group| {
            let lines = merge_edges(
                &g.coords,
                &group
                    .edges
                    .into_iter()
                    .map(|id| g.edges[id].nodes)
                    .collect::<Vec<_>>(),
            );
            let geometry = if lines.len() == 1 {
                Geometry::LineString(lines.into_iter().next().unwrap_or_else(LineString::empty))
            } else {
                MultiLineString(lines).into()
            };
            Violation {
                rule: spec.rule,
                kind: if self_rule {
                    ViolationKind::SelfOverlap
                } else {
                    ViolationKind::Overlap
                },
                geometry,
                origin: Some(group.a),
                destination: group.b,
            }
        })
        .collect()
}
fn meetings(g: &LineGraph, spec: RuleSpec, touch: bool) -> Vec<Violation> {
    let mut out = Vec::new();
    for (node, uses) in g.incident.iter().enumerate() {
        let mut features: BTreeMap<Feature, Meeting> = BTreeMap::new();
        for usage in uses {
            let part = &g.parts[usage.part];
            let reference = part.reference;
            let endpoint = part.endpoint(node);
            let entry = features.entry(reference.feature()).or_insert(Meeting {
                reference,
                interior: false,
                endpoints: 0,
            });
            entry.reference.aggregate(reference);
            entry.interior |= !endpoint;
            entry.endpoints += usize::from(endpoint);
        }
        let features = features
            .into_values()
            .map(|mut m| {
                m.interior |= m.endpoints.is_multiple_of(2);
                m
            })
            .collect::<Vec<_>>();
        for i in 0..features.len() {
            for j in i + 1..features.len() {
                let a = features[i];
                let b = features[j];
                let Some((origin, destination)) = selected(spec, a.reference, b.reference) else {
                    continue;
                };
                if !(a.interior && b.interior || touch && (a.interior || b.interior)) {
                    continue;
                }
                // A shared edge is already an overlap, not an extra crossing at every vertex.
                if uses.iter().any(|usage| {
                    let edge = &g.edges[usage.edge];
                    edge.uses
                        .iter()
                        .any(|u| g.parts[u.part].reference.feature() == a.reference.feature())
                        && edge
                            .uses
                            .iter()
                            .any(|u| g.parts[u.part].reference.feature() == b.reference.feature())
                }) {
                    continue;
                }
                out.push(point(
                    spec,
                    if a.interior && b.interior {
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
    }
    out
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
fn self_crossings(g: &LineGraph, spec: RuleSpec) -> Vec<Violation> {
    type Passage = (FeatureRef, [usize; 2]);
    let mut visits: BTreeMap<(usize, Feature), Vec<Passage>> = BTreeMap::new();
    for part in &g.parts {
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
