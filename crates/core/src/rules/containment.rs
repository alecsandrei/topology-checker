//! Whole-feature containment between layers of any geometry family.
use super::violation;
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{cross, index},
};
use geo::Relate;
use rayon::prelude::*;

/// Report cleaned features topologically contained by a target feature, excluding
/// mutual containment.
pub(super) fn contained(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    let av = cross::views(t.parts(spec.layer)?);
    let bv = cross::views(t.parts(other)?);
    let violations = index::pairs(&av.feature_index, &bv.feature_index, 0., false)
        .par_iter()
        .filter_map(|&(i, j)| {
            let (a, b) = (&av.features[i], &bv.features[j]);
            let relation = b.geometry.relate(&a.geometry);
            (relation.is_contains() && !relation.is_within()).then(|| {
                violation(
                    spec,
                    ViolationKind::Contained,
                    a.geometry.clone(),
                    Some(a.reference.at(spec.layer)),
                    Some(b.reference.at(other)),
                )
            })
        })
        .collect();
    Ok(aggregate(violations, Merge::AllDimensions))
}
