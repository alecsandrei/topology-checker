use crate::{Error, FeatureRef, RuleSpec, Topology, Violation, ViolationKind, model::part_count};
pub(crate) fn multipart(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
    Ok(t.layer(spec.layer)?
        .features()
        .filter(|(_, g)| part_count(g) > 1)
        .map(|(fid, g)| Violation {
            rule: spec.rule,
            kind: ViolationKind::Multipart,
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
