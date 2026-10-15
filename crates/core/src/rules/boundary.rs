//! Shared linework between lines and individual polygon exterior and hole boundaries.
use super::{selected_pieces, violation};
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{cross, index, line_graph::InputMode, proximity},
};
use geo::{BoundingRect, Coord, Euclidean, Length, Line};
use num_rational::BigRational as Q;
use num_traits::{One, Zero};
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
    if mode != Boundary::NotShared {
        return uncovered(t, spec, other, mode);
    }
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

fn uncovered(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    mode: Boundary,
) -> Result<Vec<Violation>, Error> {
    let source = t.parts(spec.layer)?;
    let target = t.parts(other)?;
    let source_lines = if mode == Boundary::BoundariesCovered {
        cross::boundaries(source)
    } else {
        source.lines.as_slice()
    };
    let mut violations = Vec::new();
    for source in source_lines {
        crate::cancel::check();
        let mut coords = Vec::<Coord>::new();
        let mut selected = Vec::new();
        for segment in source.geometry.lines() {
            let query = index::expanded(segment.bounding_rect(), t.tolerance());
            let mut covered = Vec::new();
            let mut cover = |boundary| {
                covered.extend(proximity::capsule(segment, boundary, t.tolerance()));
            };
            if mode == Boundary::BoundariesCovered {
                let view = cross::segments(target);
                for item in view.index.locate_in_envelope_intersecting(&query) {
                    cover(view.segments[item.data].geometry);
                }
            } else {
                let locations = crate::engine::location::locations(target);
                for item in target
                    .polygon_index()
                    .locate_in_envelope_intersecting(&query)
                {
                    let location = &locations[item.data];
                    for edge in location.tree.locate_in_envelope_intersecting(&query) {
                        cover(location.segments[edge.data]);
                    }
                }
            }
            let covered = proximity::union(covered);
            for [start, end] in proximity::outside(Q::zero(), Q::one(), &covered) {
                append(
                    &mut coords,
                    &mut selected,
                    segment,
                    start.outward(segment, false),
                    end.outward(segment, true),
                    true,
                );
            }
            // Covered intervals keep residual runs separate and preserve source
            // vertices. No new path is inserted into the original geometry.
            let start = coords.last().copied().unwrap_or(segment.start);
            append(
                &mut coords,
                &mut selected,
                segment,
                start,
                segment.end,
                false,
            );
        }
        violations.extend(
            selected_pieces(&coords, &selected)
                .into_iter()
                .map(|piece| {
                    violation(
                        spec,
                        ViolationKind::NotCovered,
                        piece,
                        Some(source.reference.at(spec.layer)),
                        None,
                    )
                }),
        );
    }
    Ok(aggregate(violations, Merge::AllDimensions))
}

fn append(
    coords: &mut Vec<Coord>,
    selected: &mut Vec<bool>,
    segment: Line,
    start: Coord,
    end: Coord,
    keep: bool,
) {
    if coords.is_empty() {
        coords.push(segment.start);
    }
    if coords.last() != Some(&start) {
        coords.push(start);
        selected.push(false);
    }
    if coords.last() != Some(&end) {
        coords.push(end);
        selected.push(keep);
    }
}
