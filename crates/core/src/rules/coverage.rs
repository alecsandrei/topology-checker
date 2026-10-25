//! Required and prohibited proximity of points and line endpoints to other features.
use super::violation;
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{cross, index, proximity},
    model::{Part, Parts},
};
use geo::{Line, Point};

/// Secondary-layer features that cover a point within tolerance.
#[derive(Debug, Clone, Copy)]
pub(super) enum Target {
    /// Any segment of the linework.
    Linework,
    /// An endpoint of an open line part.
    LineEndpoints,
    /// An exterior or hole boundary segment, including shared edges.
    PolygonBoundaries,
    /// A point part.
    Points,
}

fn on_segments(
    point: &Point,
    segments: &[Part<Line>],
    tree: &index::Index,
    tolerance: f64,
) -> bool {
    tree.locate_in_envelope_intersecting(&index::at(point.0, tolerance))
        .any(|i| proximity::near_segment(point.0, segments[i.data].geometry, tolerance))
}
fn on_points(point: &Point, points: &[Part<Point>], tree: &index::Packed, tolerance: f64) -> bool {
    tree.locate(index::at(point.0, tolerance))
        .any(|i| proximity::coincident(points[i.id].geometry.0, point.0, tolerance))
}
fn covered(point: &Point, target: Target, parts: &Parts, tolerance: f64) -> bool {
    match target {
        Target::Linework => {
            let view = cross::segments(parts);
            on_segments(point, &view.segments, &view.index, tolerance)
        }
        Target::LineEndpoints => {
            let view = cross::endpoints(parts);
            view.index
                .locate_in_envelope_intersecting(&index::at(point.0, tolerance))
                .any(|i| {
                    view.is_open(i.data, tolerance)
                        && proximity::coincident(view.points[i.data].geometry.0, point.0, tolerance)
                })
        }
        Target::PolygonBoundaries => {
            let locations = crate::engine::location::locations(parts);
            parts
                .polygon_index()
                .locate_in_envelope_intersecting(&index::at(point.0, tolerance))
                .any(|item| locations[item.data].near(point.0, tolerance))
        }
        Target::Points => on_points(point, &parts.points, parts.point_index(), tolerance),
    }
}
fn uncovered(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    target: Target,
    sources: &[Part<Point>],
    endpoints: Option<&cross::Endpoints>,
) -> Result<Vec<Violation>, Error> {
    let b = t.parts(other)?;
    let violations = super::flat_map(sources.len(), |i| {
        let p = &sources[i];
        t.cancellable(|| {
            endpoints.is_none_or(|ends| ends.is_open(i, t.tolerance()))
                && !covered(&p.geometry, target, b, t.tolerance())
        })
        .then(|| {
            violation(
                spec,
                ViolationKind::NotCovered,
                p.geometry,
                Some(p.reference.at(spec.layer)),
                None,
            )
        })
    });
    Ok(aggregate(violations, Merge::AllDimensions))
}

/// Report point parts that no target covers.
pub(super) fn points(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    target: Target,
) -> Result<Vec<Violation>, Error> {
    uncovered(t, spec, other, target, &t.parts(spec.layer)?.points, None)
}
/// Report open line part endpoints that no target covers.
pub(super) fn endpoints(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    target: Target,
) -> Result<Vec<Violation>, Error> {
    let a = cross::endpoints(t.parts(spec.layer)?);
    uncovered(t, spec, other, target, &a.points, Some(a))
}
/// Report each point within tolerance of a line part.
pub(super) fn line_contacts(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    let a = t.parts(spec.layer)?;
    let b = t.parts(other)?;
    let pairs = index::nearby(a.point_order(), b.line_index(), t.tolerance());
    let contact = |(i, j): (usize, usize)| {
        let (p, l) = (&a.points[i], &b.lines[j]);
        t.cancellable(|| {
            l.geometry
                .lines()
                .any(|line| proximity::near_segment(p.geometry.0, line, t.tolerance()))
        })
        .then(|| {
            violation(
                spec,
                ViolationKind::Intersection,
                p.geometry,
                Some(p.reference.at(spec.layer)),
                Some(l.reference.at(other)),
            )
        })
    };
    Ok(aggregate(
        super::each_pair(pairs, contact),
        Merge::AllDimensions,
    ))
}
