use crate::{
    Error, ErrorType, RuleSpec, Topology, TopologyError,
    engine::{index, polygons},
    topology::aggregate,
};
use geo::{
    BooleanOps, BoundingRect, Euclidean, Geometry, Length, MultiLineString, MultiPolygon, Polygon,
};
use rayon::prelude::*;

pub(crate) fn overlap(t: &Topology, spec: RuleSpec) -> Result<Vec<TopologyError>, Error> {
    let a = t.parts(spec.layer)?;
    let b = t.parts(spec.other.unwrap_or(spec.layer))?;
    let pairs = index::pairs(&a.polygon_index, &b.polygon_index, 0., spec.other.is_none());
    let errors = pairs
        .par_iter()
        .flat_map_iter(|&(i, j)| {
            let a = &a.polygons[i];
            let b = &b.polygons[j];
            let intersection = if a.reference.feature() != b.reference.feature() {
                a.geometry.intersection(&b.geometry)
            } else {
                MultiPolygon(Vec::new())
            };
            intersection
                .0
                .into_iter()
                .filter(|p| polygons::substantial(p, t.tolerance()))
                .map(|p| TopologyError {
                    rule: spec.rule,
                    error_type: ErrorType::Overlap,
                    geometry: p.into(),
                    origin: Some(a.reference),
                    destination: Some(b.reference),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    Ok(aggregate(errors))
}
pub(crate) fn gaps(t: &Topology, spec: RuleSpec) -> Result<Vec<TopologyError>, Error> {
    let union = polygons::union(t.parts(spec.layer)?);
    let mut errors = Vec::new();
    for p in &union.geometry {
        errors.push(TopologyError {
            rule: spec.rule,
            error_type: ErrorType::OuterBoundary,
            geometry: p.exterior().clone().into(),
            origin: None,
            destination: None,
        });
        for hole in p.interiors() {
            if polygons::substantial(&Polygon::new(hole.clone(), Vec::new()), t.tolerance()) {
                errors.push(TopologyError {
                    rule: spec.rule,
                    error_type: ErrorType::Gap,
                    geometry: hole.clone().into(),
                    origin: None,
                    destination: None,
                });
            }
        }
    }
    Ok(errors)
}
pub(crate) fn line_inside(t: &Topology, spec: RuleSpec) -> Result<Vec<TopologyError>, Error> {
    let a = t.parts(spec.layer)?;
    let other = spec.other.ok_or(Error::Inputs {
        rule: spec.rule,
        expected: 2,
    })?;
    let union = polygons::union(t.parts(other)?);
    Ok(a.lines
        .par_iter()
        .flat_map_iter(|line| {
            let Some(rect) = line.geometry.bounding_rect() else {
                return Vec::new();
            };
            let mut candidates = union
                .index
                .locate_in_envelope_intersecting(&index::envelope(rect))
                .map(|i| i.data)
                .collect::<Vec<_>>();
            candidates.sort_unstable();
            let outside = if candidates.is_empty() {
                MultiLineString(vec![line.geometry.clone()])
            } else {
                let polygons = MultiPolygon(
                    candidates
                        .into_iter()
                        .map(|i| union.geometry.0[i].clone())
                        .collect(),
                );
                polygons.clip(&MultiLineString(vec![line.geometry.clone()]), true)
            };
            outside
                .0
                .into_iter()
                .filter(|piece| Euclidean.length(piece) > t.tolerance())
                .map(|piece| TopologyError {
                    rule: spec.rule,
                    error_type: ErrorType::Outside,
                    geometry: Geometry::LineString(piece),
                    origin: Some(line.reference),
                    destination: None,
                })
                .collect::<Vec<_>>()
        })
        .collect())
}
