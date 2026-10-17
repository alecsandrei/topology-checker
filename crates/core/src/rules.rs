//! Rule implementations. [`run`] is the only dispatch on [`Rule`]: each arm calls a
//! rule family with a typed mode, so adding a rule cannot fall through a nested match.
mod boundary;
mod containment;
mod coverage;
mod geometry;
mod line;
mod line_polygon;
mod point;
mod point_polygon;
mod polygon;
use crate::{Error, FeatureRef, Rule, RuleSpec, Topology, Violation, ViolationKind};
use boundary::Boundary;
use coverage::Target;
use geo::{Coord, Geometry, LineString};
use line::LineCheck;
use line_polygon::LinePolygon;
use point_polygon::{PointCount, PointTest};

pub(crate) fn run(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
    // Registration guarantees a secondary layer for every binary rule.
    let other = || {
        spec.other.ok_or(Error::Inputs {
            rule: spec.rule,
            expected: 2,
        })
    };
    match spec.rule {
        Rule::GeometryMustNotBeMultipart => geometry::multipart(t, spec),
        Rule::PointMustNotOverlap | Rule::PointMustNotOverlapWith => point::overlap(t, spec),
        Rule::PointMustBeInside => point::inside(t, spec, other()?),
        Rule::PolygonMustNotOverlap | Rule::PolygonMustNotOverlapWith => polygon::overlap(t, spec),
        Rule::PolygonMustNotHaveGaps => polygon::gaps(t, spec),
        Rule::LineMustBeInside => polygon::line_inside(t, spec, other()?),
        Rule::LineMustNotHaveDangles => line::run(t, spec, LineCheck::Dangles),
        Rule::LineMustNotHavePseudoNodes => line::run(t, spec, LineCheck::PseudoNodes),
        Rule::LineMustNotOverlap | Rule::LineMustNotOverlapWith => {
            line::run(t, spec, LineCheck::Overlap)
        }
        Rule::LineMustNotSelfOverlap => line::run_self(t, spec, false),
        Rule::LineMustNotSelfIntersect => line::run_self(t, spec, true),
        Rule::LineMustNotIntersect | Rule::LineMustNotIntersectWith => {
            line::run(t, spec, LineCheck::Intersect { touch: false })
        }
        Rule::LineMustNotIntersectOrTouchInterior
        | Rule::LineMustNotIntersectOrTouchInteriorWith => {
            line::run(t, spec, LineCheck::Intersect { touch: true })
        }
        Rule::PointMustBeCoveredByLines => coverage::points(t, spec, other()?, Target::Linework),
        Rule::PointMustBeCoveredByLineEndpoints => {
            coverage::points(t, spec, other()?, Target::LineEndpoints)
        }
        Rule::PointMustBeCoveredByPolygonBoundaries => {
            coverage::points(t, spec, other()?, Target::PolygonBoundaries)
        }
        Rule::LineEndpointsMustBeCoveredByPoints => {
            coverage::endpoints(t, spec, other()?, Target::Points)
        }
        Rule::LineEndpointsMustBeCoveredByPolygonBoundaries => {
            coverage::endpoints(t, spec, other()?, Target::PolygonBoundaries)
        }
        Rule::PointMustNotIntersectLines => coverage::line_contacts(t, spec, other()?),
        Rule::PointMustBeCoveredByPolygons => {
            point_polygon::points(t, spec, other()?, PointTest::Covered)
        }
        Rule::PointMustNotBeInsidePolygons => {
            point_polygon::points(t, spec, other()?, PointTest::NotInside)
        }
        Rule::PointMustNotIntersectPolygons => {
            point_polygon::points(t, spec, other()?, PointTest::NotIntersecting)
        }
        Rule::PolygonMustNotContainPoints => point_polygon::contained_points(t, spec, other()?),
        Rule::PolygonMustContainPoint => {
            point_polygon::point_counts(t, spec, other()?, PointCount::AtLeastOne)
        }
        Rule::PolygonMustContainOnePoint => {
            point_polygon::point_counts(t, spec, other()?, PointCount::ExactlyOne)
        }
        Rule::GeometryMustNotBeContainedBy => containment::contained(t, spec, other()?),
        Rule::LineMustBeCoveredByPolygonBoundaries => {
            boundary::uncovered(t, spec, other()?, Boundary::LinesCovered)
        }
        Rule::PolygonBoundariesMustBeCoveredByLines => {
            boundary::uncovered(t, spec, other()?, Boundary::BoundariesCovered)
        }
        Rule::LineMustNotOverlapPolygonBoundaries => boundary::shared(t, spec, other()?),
        Rule::LineMustBeProperlyInsidePolygons => line_polygon::properly_inside(t, spec, other()?),
        Rule::LineMustNotCrossPolygonBoundaries => {
            line_polygon::run(t, spec, other()?, LinePolygon::Cross)
        }
        Rule::LineMustNotOverlapWithPolygons => {
            line_polygon::run(t, spec, other()?, LinePolygon::Overlap)
        }
        Rule::LineMustNotIntersectPolygons => {
            line_polygon::run(t, spec, other()?, LinePolygon::Intersect)
        }
    }
}

fn violation(
    spec: RuleSpec,
    kind: ViolationKind,
    geometry: impl Into<Geometry>,
    origin: Option<FeatureRef>,
    destination: Option<FeatureRef>,
) -> Violation {
    Violation {
        rule: spec.rule,
        kind,
        geometry: geometry.into(),
        origin,
        destination,
    }
}
/// Extract connected selected pieces before applying the length threshold.
/// A closed path's first and last runs form one piece across the traversal seam.
fn selected_pieces(coords: &[Coord], selected: &[bool]) -> Vec<LineString> {
    selected_runs(coords, selected)
        .into_iter()
        .map(|run| LineString(run.into_iter().map(|i| coords[i]).collect()))
        .collect()
}
/// Coordinate indices of each piece of `selected_pieces`.
fn selected_runs(coords: &[Coord], selected: &[bool]) -> Vec<Vec<usize>> {
    let mut runs = Vec::new();
    let mut run = Vec::new();
    for (i, &keep) in selected.iter().enumerate() {
        if keep {
            if run.is_empty() {
                run.push(i);
            }
            run.push(i + 1);
        } else if !run.is_empty() {
            runs.push(std::mem::take(&mut run));
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    if runs.len() > 1
        && coords.first() == coords.last()
        && selected.first() == Some(&true)
        && selected.last() == Some(&true)
    {
        let first = runs.remove(0);
        let last = runs.last_mut().expect("two runs before joining");
        last.extend(first.into_iter().skip(1));
    }
    runs
}

/// Emit deterministic aggregated violations, with a bounded duplicate-pair path.
pub(crate) fn run_into<E>(
    t: &Topology,
    spec: RuleSpec,
    mut sink: impl FnMut(Violation) -> Result<(), E>,
) -> Result<(), crate::SinkError<E>> {
    if matches!(
        spec.rule,
        Rule::PointMustNotOverlap | Rule::PointMustNotOverlapWith
    ) {
        point::overlap_into(t, spec, sink)
    } else if matches!(
        spec.rule,
        Rule::PolygonMustNotHaveGaps
            | Rule::GeometryMustNotBeMultipart
            | Rule::PolygonMustContainPoint
            | Rule::PolygonMustContainOnePoint
            | Rule::LineMustNotHaveDangles
            | Rule::LineMustNotHavePseudoNodes
    ) {
        emit(run(t, spec)?, &mut sink)
    } else {
        // Geometry scratch belongs to one original feature and its candidate
        // targets. No complete feature-pair report is retained.
        let mut snapshot = t.snapshot();
        let mut fids = t
            .layer(spec.layer)?
            .features()
            .map(|(fid, _)| fid)
            .collect::<Vec<_>>();
        fids.sort_unstable();
        let kinds: &[ViolationKind] = match spec.rule {
            Rule::LineMustNotSelfIntersect => {
                &[ViolationKind::SelfOverlap, ViolationKind::SelfCrossing]
            }
            Rule::LineMustNotIntersect | Rule::LineMustNotIntersectWith => {
                &[ViolationKind::Overlap, ViolationKind::Crossing]
            }
            Rule::LineMustNotIntersectOrTouchInterior
            | Rule::LineMustNotIntersectOrTouchInteriorWith => &[
                ViolationKind::Overlap,
                ViolationKind::Crossing,
                ViolationKind::TouchInterior,
            ],
            _ => &[],
        };
        for kind in if kinds.is_empty() {
            vec![None]
        } else {
            kinds.iter().copied().map(Some).collect()
        } {
            for &fid in &fids {
                crate::cancel::check();
                snapshot.select_feature(spec.layer, fid)?;
                let mut violations = run(&snapshot, spec)?;
                if let Some(kind) = kind {
                    violations.retain(|v| v.kind == kind);
                }
                emit(violations, &mut sink)?;
            }
        }
        Ok(())
    }
}
fn emit<E>(
    mut violations: Vec<Violation>,
    sink: &mut impl FnMut(Violation) -> Result<(), E>,
) -> Result<(), crate::SinkError<E>> {
    violations.sort_by_key(|v| {
        (
            v.kind,
            v.origin.map(FeatureRef::feature),
            v.destination.map(FeatureRef::feature),
        )
    });
    for v in violations {
        crate::cancel::check();
        sink(v).map_err(crate::SinkError::Consumer)?;
    }
    Ok(())
}

fn feature_geometry(
    t: &Topology,
    spec: RuleSpec,
    layer: crate::LayerId,
    feature: &crate::engine::cross::Feature,
) -> Result<Geometry, Error> {
    t.cancellable(|| feature.output()).map_err(|e| {
        e.precision_context(
            "polygon feature emission",
            spec.rule,
            format!(
                ", layer {}, FID {}",
                t.layer(layer).expect("registered layer").name(),
                feature.reference.fid
            ),
        )
    })
}
