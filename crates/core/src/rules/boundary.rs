//! Shared linework between lines and individual polygon exterior and hole boundaries.
use super::{selected_pieces, violation};
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::line_graph::InputMode,
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
    let mut coverage = vec![Vec::new(); graph.parts.len()];
    for m in graph.matches() {
        let [a, b] = m.sources.map(|s| graph.segment(s));
        let (ar, br) = (graph.parts[a.part].reference, graph.parts[b.part].reference);
        if ar.layer == br.layer {
            continue;
        }
        for (s, span, target) in [(a, m.spans[0], br), (b, m.spans[1], ar)] {
            coverage[s.part].push((s.start as f64 + span[0], s.start as f64 + span[1], target));
        }
    }
    let mut violations = Vec::new();
    for (part_id, part) in graph.parts.iter().enumerate() {
        crate::cancel::check();
        if part.reference.layer != spec.layer
            || t.selected_feature()
                .is_some_and(|key| key != part.reference.feature())
        {
            continue;
        }
        let targets = &coverage[part_id];
        let destinations = if overlap {
            targets
                .iter()
                .map(|x| x.2)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(Some)
                .collect::<Vec<_>>()
        } else {
            vec![None]
        };
        let original = graph.original(part_id);
        let mut cuts = (0..original.0.len()).map(|i| i as f64).collect::<Vec<_>>();
        cuts.extend(targets.iter().flat_map(|&(a, b, _)| [a, b]));
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let coords = cuts
            .iter()
            .map(|&t| {
                let i = (t.floor() as usize).min(original.0.len() - 2);
                crate::engine::matching::point(
                    geo::Line::new(original.0[i], original.0[i + 1]),
                    t - i as f64,
                )
            })
            .collect::<Vec<_>>();
        for destination in destinations {
            let selected = cuts
                .windows(2)
                .map(|pair| {
                    let covered = targets.iter().any(|&(a, b, r)| {
                        pair[0] >= a && pair[1] <= b && destination.is_none_or(|d| d == r)
                    });
                    if overlap { covered } else { !covered }
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
