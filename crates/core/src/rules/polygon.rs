use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{index, polygons},
};
use geo::{
    BooleanOps, BoundingRect, Euclidean, Geometry, Length, MultiLineString, MultiPolygon, Polygon,
};
use rayon::prelude::*;

pub(crate) fn overlap(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
    let (a_layer, b_layer) = (spec.layer, spec.other.unwrap_or(spec.layer));
    let a = t.parts(a_layer)?;
    let b = t.parts(b_layer)?;
    let pairs = index::pairs(&a.polygon_index, &b.polygon_index, 0., spec.other.is_none());
    let errors = pairs
        .par_iter()
        .flat_map_iter(|&(i, j)| {
            let a = &a.polygons[i];
            let b = &b.polygons[j];
            let (origin, destination) = (a.reference.at(a_layer), b.reference.at(b_layer));
            let intersection = if origin.feature() != destination.feature() {
                a.geometry.intersection(&b.geometry)
            } else {
                MultiPolygon(Vec::new())
            };
            intersection
                .0
                .into_iter()
                .filter(|p| polygons::substantial(p, t.tolerance()))
                .map(|p| Violation {
                    rule: spec.rule,
                    kind: ViolationKind::Overlap,
                    geometry: p.into(),
                    origin: Some(origin),
                    destination: Some(destination),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    Ok(aggregate(errors, Merge::HighestDimension))
}
pub(crate) fn gaps(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
    let union = polygons::union(t.parts(spec.layer)?);
    let mut errors = Vec::new();
    for p in &union.geometry {
        errors.push(Violation {
            rule: spec.rule,
            kind: ViolationKind::OuterBoundary,
            geometry: p.exterior().clone().into(),
            origin: None,
            destination: None,
        });
        for hole in p.interiors() {
            if polygons::substantial(&Polygon::new(hole.clone(), Vec::new()), t.tolerance()) {
                errors.push(Violation {
                    rule: spec.rule,
                    kind: ViolationKind::Gap,
                    geometry: hole.clone().into(),
                    origin: None,
                    destination: None,
                });
            }
        }
    }
    Ok(errors)
}
pub(crate) fn line_inside(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    let a = t.parts(spec.layer)?;
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
                .map(|piece| Violation {
                    rule: spec.rule,
                    kind: ViolationKind::Outside,
                    geometry: Geometry::LineString(piece),
                    origin: Some(line.reference.at(spec.layer)),
                    destination: None,
                })
                .collect::<Vec<_>>()
        })
        .collect())
}
