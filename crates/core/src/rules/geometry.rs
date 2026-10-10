use crate::{Error, ErrorType, FeatureRef, RuleSpec, Topology, TopologyError, model::part_count};
pub(crate) fn multipart(t: &Topology, spec: RuleSpec) -> Result<Vec<TopologyError>, Error> {
    Ok(t.layer(spec.layer)?
        .features()
        .filter(|(_, g)| part_count(g) > 1)
        .map(|(fid, g)| TopologyError {
            rule: spec.rule,
            error_type: ErrorType::Multipart,
            geometry: g.clone(),
            origin: Some(FeatureRef {
                layer: spec.layer,
                fid,
                part: None,
            }),
            destination: None,
        })
        .collect())
}
