mod cross;
mod geometry;
mod line;
mod point;
mod polygon;
use crate::{Error, Rule, RuleSpec, Topology, TopologyError};

pub(crate) fn run(topology: &Topology, spec: RuleSpec) -> Result<Vec<TopologyError>, Error> {
    match spec.rule {
        Rule::GeometryMustNotBeMultipart => geometry::multipart(topology, spec),
        Rule::PointMustNotOverlap | Rule::PointMustNotOverlapWith => point::overlap(topology, spec),
        Rule::PointMustBeInside => point::inside(topology, spec),
        Rule::PolygonMustNotOverlap | Rule::PolygonMustNotOverlapWith => {
            polygon::overlap(topology, spec)
        }
        Rule::PolygonMustNotHaveGaps => polygon::gaps(topology, spec),
        Rule::LineMustBeInside => polygon::line_inside(topology, spec),
        Rule::LineMustNotHaveDangles
        | Rule::LineMustNotHavePseudoNodes
        | Rule::LineMustNotOverlap
        | Rule::LineMustNotOverlapWith
        | Rule::LineMustNotSelfOverlap
        | Rule::LineMustNotIntersect
        | Rule::LineMustNotIntersectWith
        | Rule::LineMustNotIntersectOrTouchInterior
        | Rule::LineMustNotIntersectOrTouchInteriorWith
        | Rule::LineMustNotSelfIntersect => line::run(topology, spec),
        Rule::PointMustBeCoveredByLines
        | Rule::PointMustBeCoveredByLineEndpoints
        | Rule::PointMustBeCoveredByPolygonBoundaries
        | Rule::LineEndpointsMustBeCoveredByPoints
        | Rule::LineMustBeCoveredByPolygonBoundaries
        | Rule::PolygonBoundariesMustBeCoveredByLines
        | Rule::PolygonMustContainPoint
        | Rule::PolygonMustContainOnePoint
        | Rule::LineMustNotCrossPolygonBoundaries
        | Rule::GeometryMustNotBeContainedBy
        | Rule::PointMustBeCoveredByPolygons
        | Rule::PointMustNotBeInsidePolygons
        | Rule::PointMustNotIntersectLines
        | Rule::PointMustNotIntersectPolygons
        | Rule::LineMustBeProperlyInsidePolygons
        | Rule::LineMustNotOverlapWithPolygons
        | Rule::LineMustNotOverlapPolygonBoundaries
        | Rule::LineMustNotIntersectPolygons
        | Rule::LineEndpointsMustBeCoveredByPolygonBoundaries
        | Rule::PolygonMustNotContainPoints => cross::run(topology, spec),
    }
}
