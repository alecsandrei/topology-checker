use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    engine::{index, proximity::coincident},
};
use rayon::prelude::*;

pub(crate) fn overlap(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
    let mut errors = Vec::new();
    let mut collect = |v| {
        errors.push(v);
        Ok::<_, std::convert::Infallible>(())
    };
    overlap_into(t, spec, &mut collect)?;
    Ok(errors)
}

pub(crate) fn inside(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    let a = t.parts(spec.layer)?;
    let b = t.parts(other)?;
    let locations = crate::engine::location::locations(b);
    Ok(a.points
        .par_iter()
        .filter_map(|p| {
            let inside = b
                .polygon_index()
                .locate_in_envelope_intersecting(&index::at(p.geometry.0, t.tolerance()))
                .any(|item| {
                    let polygon = &locations[item.data];
                    polygon.position(p.geometry.0) == 1
                        && !polygon.near(p.geometry.0, t.tolerance())
                });
            (!inside).then_some(Violation {
                rule: spec.rule,
                kind: ViolationKind::Outside,
                geometry: p.geometry.into(),
                origin: Some(p.reference.at(spec.layer)),
                destination: None,
            })
        })
        .collect())
}

/// Aggregate and emit one feature pair at a time, preserving every duplicate pair.
pub(crate) fn overlap_into<E>(
    t: &Topology,
    spec: RuleSpec,
    mut sink: impl FnMut(Violation) -> Result<(), E>,
) -> Result<(), crate::SinkError<E>> {
    use std::collections::BTreeMap;
    let a = t.parts(spec.layer)?;
    let other = spec.other.unwrap_or(spec.layer);
    let b = t.parts(other)?;
    let mut features: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (i, p) in a.points.iter().enumerate() {
        features.entry(p.reference.fid).or_default().push(i);
    }
    let ranks = t.layer(spec.layer)?.ranks();
    for (fid, indices) in features {
        crate::cancel::check();
        let mut targets: BTreeMap<i64, crate::FeatureRef> = BTreeMap::new();
        for &i in &indices {
            let p = &a.points[i];
            for item in b
                .point_index()
                .locate_in_envelope_intersecting(&index::at(p.geometry.0, t.tolerance()))
            {
                let q = &b.points[item.data];
                if spec.other.is_none() && ranks[&q.reference.fid] <= ranks[&fid] {
                    continue;
                }
                if !coincident(p.geometry.0, q.geometry.0, t.tolerance()) {
                    continue;
                }
                let reference = q.reference.at(other);
                targets
                    .entry(q.reference.fid)
                    .and_modify(|old| old.aggregate(reference))
                    .or_insert(reference);
            }
        }
        for (target, destination) in targets {
            crate::cancel::check();
            let mut origin = None;
            let mut points = BTreeMap::new();
            for &i in &indices {
                let p = &a.points[i];
                if indices.len() == 1
                    || b.point_feature(target)
                        .iter()
                        .any(|q| coincident(p.geometry.0, q.geometry.0, t.tolerance()))
                {
                    let reference = p.reference.at(spec.layer);
                    origin.get_or_insert(reference).aggregate(reference);
                    points.insert(
                        (
                            crate::engine::cluster::bits(p.geometry.x()),
                            crate::engine::cluster::bits(p.geometry.y()),
                        ),
                        p.geometry,
                    );
                }
            }
            // Unary roles follow input order. Report the lower FID's points so the
            // location does not depend on that order.
            if spec.other.is_none() && target < fid {
                let (own, theirs) = (a.point_feature(fid), b.point_feature(target));
                points = theirs
                    .iter()
                    .filter(|q| {
                        theirs.len() == 1
                            || own
                                .iter()
                                .any(|p| coincident(p.geometry.0, q.geometry.0, t.tolerance()))
                    })
                    .map(|q| {
                        (
                            (
                                crate::engine::cluster::bits(q.geometry.x()),
                                crate::engine::cluster::bits(q.geometry.y()),
                            ),
                            q.geometry,
                        )
                    })
                    .collect();
            }
            let mut points = points.into_values().collect::<Vec<_>>();
            points.sort_by(|a, b| a.x().total_cmp(&b.x()).then(a.y().total_cmp(&b.y())));
            let geometry = if points.len() == 1 {
                points[0].into()
            } else {
                geo::MultiPoint(points).into()
            };
            sink(Violation {
                rule: spec.rule,
                kind: ViolationKind::Overlap,
                geometry,
                origin,
                destination: Some(destination),
            })
            .map_err(crate::SinkError::Consumer)?;
        }
    }
    Ok(())
}
