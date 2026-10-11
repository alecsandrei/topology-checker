//! Required and prohibited proximity of points and line endpoints to other features.
use super::violation;
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{
        cluster::distance,
        cross::{self, Views},
        index,
    },
    model::{Part, Parts},
};
use geo::{Distance, Euclidean, Line, Point};
use rayon::prelude::*;

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
        .any(|i| Euclidean.distance(&segments[i.data].geometry, point) <= tolerance)
}
fn on_points(point: &Point, points: &[Part<Point>], tree: &index::Index, tolerance: f64) -> bool {
    tree.locate_in_envelope_intersecting(&index::at(point.0, tolerance))
        .any(|i| distance(points[i.data].geometry.0, point.0) <= tolerance)
}
fn covered(point: &Point, target: Target, parts: &Parts, views: &Views, tolerance: f64) -> bool {
    match target {
        Target::Linework => on_segments(point, &views.segments, &views.segment_index, tolerance),
        Target::LineEndpoints => {
            on_points(point, &views.endpoints, &views.endpoint_index, tolerance)
        }
        Target::PolygonBoundaries => on_segments(
            point,
            &views.boundary_segments,
            &views.boundary_index,
            tolerance,
        ),
        Target::Points => on_points(point, &parts.points, &parts.point_index, tolerance),
    }
}
fn uncovered(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    target: Target,
    sources: &[Part<Point>],
) -> Result<Vec<Violation>, Error> {
    let b = t.parts(other)?;
    let bv = cross::views(b);
    let violations = sources
        .par_iter()
        .filter(|p| !covered(&p.geometry, target, b, bv, t.tolerance()))
        .map(|p| {
            violation(
                spec,
                ViolationKind::NotCovered,
                p.geometry,
                Some(p.reference.at(spec.layer)),
                None,
            )
        })
        .collect();
    Ok(aggregate(violations, Merge::AllDimensions))
}

/// Report point parts that no target covers.
pub(super) fn points(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    target: Target,
) -> Result<Vec<Violation>, Error> {
    uncovered(t, spec, other, target, &t.parts(spec.layer)?.points)
}
/// Report open line part endpoints that no target covers.
pub(super) fn endpoints(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    target: Target,
) -> Result<Vec<Violation>, Error> {
    let a = cross::views(t.parts(spec.layer)?);
    uncovered(t, spec, other, target, &a.endpoints)
}
/// Report each point within tolerance of a line part.
pub(super) fn line_contacts(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    let a = t.parts(spec.layer)?;
    let b = t.parts(other)?;
    let violations = index::pairs(&a.point_index, &b.line_index, t.tolerance(), false)
        .par_iter()
        .filter_map(|&(i, j)| {
            let (p, l) = (&a.points[i], &b.lines[j]);
            (Euclidean.distance(&l.geometry, &p.geometry) <= t.tolerance()).then(|| {
                violation(
                    spec,
                    ViolationKind::Intersection,
                    p.geometry,
                    Some(p.reference.at(spec.layer)),
                    Some(l.reference.at(other)),
                )
            })
        })
        .collect();
    Ok(aggregate(violations, Merge::AllDimensions))
}
