//! Shared linework between lines and individual polygon exterior and hole boundaries.
use super::{selected_pieces, violation};
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::line_graph::InputMode,
    rules::line::parallel,
};
use geo::{Euclidean, Length};
use std::collections::BTreeSet;

/// Which noded linework must, or must not, coincide with the other layer's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Boundary {
    /// Line pieces must lie on polygon boundaries.
    LinesCovered,
    /// Polygon boundary pieces must lie on lines.
    BoundariesCovered,
    /// Line pieces must not lie on polygon boundaries.
    NotShared,
}

/// Report primary pieces longer than tolerance that violate the coverage requirement.
pub(super) fn linework(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    mode: Boundary,
) -> Result<Vec<Violation>, Error> {
    let (primary, secondary) = if mode == Boundary::BoundariesCovered {
        (InputMode::PolygonBoundary, InputMode::Linework)
    } else {
        (InputMode::Linework, InputMode::PolygonBoundary)
    };
    let graph = t.graph_inputs(vec![(spec.layer, primary), (other, secondary)])?;
    let overlap = mode == Boundary::NotShared;
    let mut violations = Vec::new();
    let parts: Box<dyn Iterator<Item = usize> + '_> = match t.selected_feature() {
        Some(key) => Box::new(
            graph
                .feature(key)
                .into_iter()
                .flat_map(|f| f.parts.iter().copied()),
        ),
        None => Box::new(0..graph.parts.len()),
    };
    for part_id in parts {
        crate::cancel::check();
        let part = &graph.parts[part_id];
        if part.reference.layer != spec.layer {
            continue;
        }
        let targets = part
            .nodes
            .windows(2)
            .map(|pair| {
                let key = [pair[0].min(pair[1]), pair[0].max(pair[1])];
                let id = graph
                    .edges
                    .binary_search_by_key(&key, |edge| edge.nodes)
                    .expect("source edge");
                let uses = graph.edges[id].uses(&graph);
                let own_start = uses.partition_point(|u| u.part < part_id);
                let own_end = uses.partition_point(|u| u.part <= part_id);
                let other_start =
                    uses.partition_point(|u| graph.parts[u.part].reference.layer < other);
                let other_end =
                    uses.partition_point(|u| graph.parts[u.part].reference.layer <= other);
                let mut targets = BTreeSet::new();
                for target in &uses[other_start..other_end] {
                    if uses[own_start..own_end]
                        .iter()
                        .any(|u| parallel(graph.source(u.source), graph.source(target.source)))
                    {
                        targets.insert(graph.parts[target.part].reference);
                        if !overlap {
                            break;
                        }
                    }
                }
                targets
            })
            .collect::<Vec<_>>();
        let destinations: Vec<_> = if overlap {
            targets
                .iter()
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(Some)
                .collect()
        } else {
            vec![None]
        };
        for destination in destinations {
            let coords = &part.coords;
            let selected = targets
                .iter()
                .map(|covered| {
                    if let Some(r) = destination {
                        covered.contains(&r)
                    } else {
                        covered.is_empty()
                    }
                })
                .collect::<Vec<_>>();
            for piece in selected_pieces(coords, &selected) {
                if Euclidean.length(&piece) > t.tolerance() {
                    violations.push(violation(
                        spec,
                        if overlap {
                            ViolationKind::Overlap
                        } else {
                            ViolationKind::NotCovered
                        },
                        piece,
                        Some(part.reference),
                        destination,
                    ));
                }
            }
        }
    }
    Ok(aggregate(violations, Merge::AllDimensions))
}
