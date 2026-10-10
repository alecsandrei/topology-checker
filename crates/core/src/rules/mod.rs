mod geometry;
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
        _ => Err(Error::Unavailable(spec.rule)),
    }
}
