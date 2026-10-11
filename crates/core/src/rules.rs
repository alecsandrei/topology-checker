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
use geo::{Contains, Coord, Distance, Euclidean, Geometry, LineString, Point, Polygon};
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
        Rule::LineMustNotSelfOverlap => line::run(t, spec, LineCheck::SelfOverlap),
        Rule::LineMustNotSelfIntersect => line::run(t, spec, LineCheck::SelfIntersect),
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
            boundary::linework(t, spec, other()?, Boundary::LinesCovered)
        }
        Rule::PolygonBoundariesMustBeCoveredByLines => {
            boundary::linework(t, spec, other()?, Boundary::BoundariesCovered)
        }
        Rule::LineMustNotOverlapPolygonBoundaries => {
            boundary::linework(t, spec, other()?, Boundary::NotShared)
        }
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
fn boundary_distance(p: &Polygon, point: &Point) -> f64 {
    std::iter::once(p.exterior())
        .chain(p.interiors())
        .map(|r| Euclidean.distance(r, point))
        .fold(f64::INFINITY, f64::min)
}
/// Whether a point is inside a polygon and farther than `tolerance` from its boundary.
fn strict(p: &Polygon, point: &Point, tolerance: f64) -> bool {
    p.contains(point) && boundary_distance(p, point) > tolerance
}

/// Extract connected selected pieces before applying the length threshold.
/// A closed path's first and last runs form one piece across the traversal seam.
fn selected_pieces(coords: &[Coord], selected: &[bool]) -> Vec<LineString> {
    let mut pieces = Vec::new();
    let mut run = Vec::new();
    for (i, &keep) in selected.iter().enumerate() {
        if keep {
            if run.is_empty() {
                run.push(coords[i]);
            }
            run.push(coords[i + 1]);
        } else if !run.is_empty() {
            pieces.push(LineString(std::mem::take(&mut run)));
        }
    }
    if !run.is_empty() {
        pieces.push(LineString(run));
    }
    if pieces.len() > 1
        && coords.first() == coords.last()
        && selected.first() == Some(&true)
        && selected.last() == Some(&true)
    {
        let first = pieces.remove(0);
        let last = pieces.last_mut().expect("two pieces before joining");
        last.0.extend(first.0.into_iter().skip(1));
    }
    pieces
}
