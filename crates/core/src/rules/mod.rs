mod geometry;
mod point;
use crate::{Error, Rule, RuleSpec, Topology, TopologyError};

pub(crate) fn run(topology: &Topology, spec: RuleSpec) -> Result<Vec<TopologyError>, Error> {
    match spec.rule {
        Rule::GeometryMustNotBeMultipart => geometry::multipart(topology, spec),
        Rule::PointMustNotOverlap | Rule::PointMustNotOverlapWith => point::overlap(topology, spec),
        Rule::PointMustBeInside => point::inside(topology, spec),
        _ => Err(Error::Unavailable(spec.rule)),
    }
}
