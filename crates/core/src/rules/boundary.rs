//! Shared linework between lines and individual polygon exterior and hole boundaries.
use super::{selected_pieces, violation};
use crate::{
    Error, FeatureRef, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::line_graph::InputMode,
};
use geo::{Euclidean, Length};
use std::collections::{BTreeMap, BTreeSet};

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
    let mut targets: BTreeMap<_, BTreeSet<FeatureRef>> = BTreeMap::new();
    for edge in &graph.edges {
        for usage in &edge.uses {
            let r = graph.parts[usage.part].reference;
            if r.layer == other {
                targets
                    .entry((edge.nodes[0], edge.nodes[1]))
                    .or_default()
                    .insert(r);
            }
        }
    }
    let mut violations = Vec::new();
    for part in &graph.parts {
        if part.reference.layer != spec.layer {
            continue;
        }
        let destinations: Vec<_> = if overlap {
            part.nodes
                .windows(2)
                .filter_map(|pair| targets.get(&(pair[0].min(pair[1]), pair[0].max(pair[1]))))
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
            let coords = part
                .nodes
                .iter()
                .map(|&node| graph.coords[node])
                .collect::<Vec<_>>();
            let selected = part
                .nodes
                .windows(2)
                .map(|pair| {
                    let key = (pair[0].min(pair[1]), pair[0].max(pair[1]));
                    let covered = targets.get(&key);
                    if let Some(r) = destination {
                        covered.is_some_and(|set| set.contains(&r))
                    } else {
                        covered.is_none()
                    }
                })
                .collect::<Vec<_>>();
            for piece in selected_pieces(&coords, &selected) {
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
