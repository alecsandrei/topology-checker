use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{cluster::distance, index},
};
use geo::{Contains, Distance, Euclidean, Geometry};
use rayon::prelude::*;

pub(crate) fn overlap(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
    let (a_layer, b_layer) = (spec.layer, spec.other.unwrap_or(spec.layer));
    let a = t.parts(a_layer)?;
    let b = t.parts(b_layer)?;
    let candidates = index::pairs(
        &a.point_index,
        &b.point_index,
        t.tolerance(),
        spec.other.is_none(),
    );
    let errors = candidates
        .par_iter()
        .filter_map(|&(i, j)| {
            let a = &a.points[i];
            let b = &b.points[j];
            let (origin, destination) = (a.reference.at(a_layer), b.reference.at(b_layer));
            if origin.feature() == destination.feature()
                || distance(a.geometry.0, b.geometry.0) > t.tolerance()
            {
                return None;
            }
            Some(Violation {
                rule: spec.rule,
                kind: ViolationKind::Overlap,
                geometry: Geometry::Point(a.geometry),
                origin: Some(origin),
                destination: Some(destination),
            })
        })
        .collect();
    Ok(aggregate(errors, Merge::HighestDimension))
}
pub(crate) fn inside(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    let a = t.parts(spec.layer)?;
    let b = t.parts(other)?;
    Ok(a.points
        .par_iter()
        .filter_map(|p| {
            let inside = b
                .polygon_index
                .locate_in_envelope_intersecting(&index::at(p.geometry.0, t.tolerance()))
                .any(|item| {
                    let polygon = &b.polygons[item.data].geometry;
                    polygon.contains(&p.geometry)
                        && Euclidean.distance(polygon.exterior(), &p.geometry) > t.tolerance()
                        && polygon
                            .interiors()
                            .iter()
                            .all(|ring| Euclidean.distance(ring, &p.geometry) > t.tolerance())
                });
            (!inside).then_some(Violation {
                rule: spec.rule,
                kind: ViolationKind::Outside,
                geometry: Geometry::Point(p.geometry),
                origin: Some(p.reference.at(spec.layer)),
                destination: None,
            })
        })
        .collect())
}
