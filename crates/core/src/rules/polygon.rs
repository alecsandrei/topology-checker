use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    engine::{arrangement, index, polygons},
};
use rayon::prelude::*;
use std::collections::BTreeMap;

pub(crate) fn overlap(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
    let (a_layer, b_layer) = (spec.layer, spec.other.unwrap_or(spec.layer));
    let a = t.parts(a_layer)?;
    let b = t.full_parts(b_layer)?;
    let ranks = t.layer(spec.layer)?.ranks();
    let mut groups = BTreeMap::<(i64, i64), (Vec<usize>, Vec<usize>)>::new();
    for (i, j) in index::pairs(
        a.polygon_index(),
        b.polygon_index(),
        0.,
        spec.other.is_none() && t.selected_feature().is_none(),
    ) {
        let (ap, bp) = (&a.polygons[i], &b.polygons[j]);
        if ap.reference.at(a_layer).feature() == bp.reference.at(b_layer).feature() {
            continue;
        }
        if t.selected_feature().is_some()
            && spec.other.is_none()
            && ranks[&bp.reference.fid] <= ranks[&ap.reference.fid]
        {
            continue;
        }
        let (ai, bj) = groups
            .entry((ap.reference.fid, bp.reference.fid))
            .or_default();
        ai.push(i);
        bj.push(j);
    }
    groups
        .into_par_iter()
        .map(|((afid, bfid), (mut ai, mut bj))| {
            ai.sort_unstable();
            ai.dedup();
            bj.sort_unstable();
            bj.dedup();
            let geometry = t
                .cancellable(|| {
                    arrangement::overlay(
                        ai.iter().map(|&i| a.polygons[i].geometry.as_ref()),
                        bj.iter().map(|&j| b.polygons[j].geometry.as_ref()),
                        true,
                    )
                    .emit(t.tolerance())
                })
                .map_err(|e| {
                    e.precision_context(
                        "polygon intersection emission",
                        spec.rule,
                        format!(
                            ", layer {}, FID {afid}; layer {}, FID {bfid}",
                            t.layer(a_layer).expect("registered layer").name(),
                            t.layer(b_layer).expect("registered layer").name()
                        ),
                    )
                })?;
            if geometry.0.is_empty() {
                return Ok(None);
            }
            // Bounding-box candidates that only touch must not clear original
            // part provenance. Classify contribution on exact original operands.
            let contributing_a = if ai.len() == 1 {
                ai.clone()
            } else {
                ai.iter()
                    .copied()
                    .filter(|&i| {
                        t.cancellable(|| {
                            arrangement::overlay(
                                [a.polygons[i].geometry.as_ref()],
                                bj.iter().map(|&j| b.polygons[j].geometry.as_ref()),
                                true,
                            )
                            .has_area()
                        })
                    })
                    .collect()
            };
            let contributing_b = if bj.len() == 1 {
                bj.clone()
            } else {
                bj.iter()
                    .copied()
                    .filter(|&j| {
                        t.cancellable(|| {
                            arrangement::overlay(
                                ai.iter().map(|&i| a.polygons[i].geometry.as_ref()),
                                [b.polygons[j].geometry.as_ref()],
                                true,
                            )
                            .has_area()
                        })
                    })
                    .collect()
            };
            let mut origin = a.polygons[contributing_a[0]].reference.at(a_layer);
            let mut destination = b.polygons[contributing_b[0]].reference.at(b_layer);
            for &i in &contributing_a[1..] {
                origin.aggregate(a.polygons[i].reference.at(a_layer));
            }
            for &j in &contributing_b[1..] {
                destination.aggregate(b.polygons[j].reference.at(b_layer));
            }
            let geometry = if geometry.0.len() == 1 {
                geometry.0.into_iter().next().expect("one polygon").into()
            } else {
                geometry.into()
            };
            Ok(Some(Violation {
                rule: spec.rule,
                kind: ViolationKind::Overlap,
                geometry,
                origin: Some(origin),
                destination: Some(destination),
            }))
        })
        .collect::<Result<Vec<_>, Error>>()
        .map(|v| v.into_iter().flatten().collect())
}
pub(crate) fn gaps(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
    let union = polygons::union(t.parts(spec.layer)?);
    let rings = union.geometry.gaps(t.tolerance()).map_err(|e| {
        e.precision_context(
            "polygon union emission",
            spec.rule,
            format!(
                ", layer {}, FIDs {:?}",
                t.layer(spec.layer).expect("registered layer").name(),
                t.layer(spec.layer)
                    .expect("registered layer")
                    .features()
                    .map(|(fid, _)| fid)
                    .collect::<Vec<_>>()
            ),
        )
    })?;
    Ok(rings
        .into_iter()
        .map(|(hole, ring)| Violation {
            rule: spec.rule,
            kind: if hole {
                ViolationKind::Gap
            } else {
                ViolationKind::OuterBoundary
            },
            geometry: ring.into(),
            origin: None,
            destination: None,
        })
        .collect())
}
pub(crate) fn line_inside(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    super::line_polygon::inside(t, spec, other)
}
