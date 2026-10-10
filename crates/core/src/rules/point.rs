use crate::{
    Error, ErrorType, RuleSpec, Topology, TopologyError,
    engine::{cluster::distance, index},
    topology::aggregate,
};
use geo::{Contains, Distance, Euclidean, Geometry};
use rayon::prelude::*;

pub(crate) fn overlap(t: &Topology, spec: RuleSpec) -> Result<Vec<TopologyError>, Error> {
    let a = t.parts(spec.layer)?;
    let b = t.parts(spec.other.unwrap_or(spec.layer))?;
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
            if a.reference.feature() == b.reference.feature()
                || distance(a.geometry.0, b.geometry.0) > t.tolerance()
            {
                return None;
            }
            Some(TopologyError {
                rule: spec.rule,
                error_type: ErrorType::Overlap,
                geometry: Geometry::Point(a.geometry),
                origin: Some(a.reference),
                destination: Some(b.reference),
            })
        })
        .collect();
    Ok(aggregate(errors))
}
pub(crate) fn inside(t: &Topology, spec: RuleSpec) -> Result<Vec<TopologyError>, Error> {
    let a = t.parts(spec.layer)?;
    let other = spec.other.ok_or(Error::Inputs {
        rule: spec.rule,
        expected: 2,
    })?;
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
            (!inside).then_some(TopologyError {
                rule: spec.rule,
                error_type: ErrorType::Outside,
                geometry: Geometry::Point(p.geometry),
                origin: Some(p.reference),
                destination: None,
            })
        })
        .collect())
}
