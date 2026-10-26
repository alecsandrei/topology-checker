//! Whole-feature containment between layers of any geometry family.
use super::violation;
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{cross, index},
};
use geo::Relate;

/// Report cleaned features topologically contained by a target feature, excluding
/// mutual containment.
pub(super) fn contained(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    let av = cross::views(t.parts(spec.layer)?);
    let bv = cross::views(t.parts(other)?);
    let pairs = index::pairs(&av.feature_index, &bv.feature_index, 0., false);
    let contained = |(i, j): (usize, usize)| {
        let (a, b) = (&av.features[i], &bv.features[j]);
        let contained = t.cancellable(|| {
            if b.is_areal()
                && matches!(
                    a.geometry,
                    geo::Geometry::Point(_) | geo::Geometry::MultiPoint(_)
                )
            {
                let positions = a.points.iter().map(|p| b.position(p.0)).collect::<Vec<_>>();
                positions.iter().all(|&p| p >= 0) && positions.contains(&1)
            } else {
                let relation = b.geometry.relate(&a.geometry);
                relation.is_contains() && !relation.is_within()
            }
        });
        if !contained {
            return Ok(None);
        }
        let geometry = super::feature_geometry(t, spec, spec.layer, a)?;
        Ok(Some(violation(
            spec,
            ViolationKind::Contained,
            geometry,
            Some(a.reference.at(spec.layer)),
            Some(b.reference.at(other)),
        )))
    };
    let violations = super::each_pair(pairs, |pair| contained(pair).transpose())
        .into_iter()
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(aggregate(violations, Merge::AllDimensions))
}
