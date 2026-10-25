//! Relations between point features and polygon areas.
use super::violation;
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{cross, index},
};

/// How each point part relates to the polygon layer.
#[derive(Debug, Clone, Copy)]
pub(super) enum PointTest {
    /// Each point must be inside a polygon or within tolerance of its boundary.
    Covered,
    /// No point may be strictly inside a polygon, farther than tolerance from its boundary.
    NotInside,
    /// No point may be inside a polygon or within tolerance of its boundary.
    NotIntersecting,
}
/// How many point features each polygon feature must strictly contain.
#[derive(Debug, Clone, Copy)]
pub(super) enum PointCount {
    /// At least one; uncontained points are allowed.
    AtLeastOne,
    /// Exactly one, and every point feature belongs to exactly one polygon feature.
    ExactlyOne,
}

/// Test every point part against the polygon parts near it.
pub(super) fn points(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    test: PointTest,
) -> Result<Vec<Violation>, Error> {
    let a = t.parts(spec.layer)?;
    let b = t.parts(other)?;
    let locations = crate::engine::location::locations(b);
    let violations = super::flat_map(a.points.len(), |point| {
        let p = &a.points[point];
        let hits = t.cancellable(|| {
            let mut candidates = b
                .polygon_index()
                .locate_in_envelope_intersecting(&index::at(p.geometry.0, t.tolerance()))
                .map(|i| i.data)
                .collect::<Vec<_>>();
            candidates.sort_unstable();
            candidates
                .into_iter()
                .filter(|&i| {
                    let polygon = &locations[i];
                    match test {
                        PointTest::NotInside => {
                            polygon.position(p.geometry.0) == 1
                                && !polygon.near(p.geometry.0, t.tolerance())
                        }
                        PointTest::Covered | PointTest::NotIntersecting => {
                            polygon.position(p.geometry.0) >= 0
                                || polygon.near(p.geometry.0, t.tolerance())
                        }
                    }
                })
                .collect::<Vec<_>>()
        });
        let origin = Some(p.reference.at(spec.layer));
        let kind = match test {
            PointTest::Covered => {
                return if hits.is_empty() {
                    vec![violation(
                        spec,
                        ViolationKind::NotCovered,
                        p.geometry,
                        origin,
                        None,
                    )]
                } else {
                    Vec::new()
                };
            }
            PointTest::NotInside => ViolationKind::Contained,
            PointTest::NotIntersecting => ViolationKind::Intersection,
        };
        hits.into_iter()
            .map(|i| {
                violation(
                    spec,
                    kind,
                    p.geometry,
                    origin,
                    Some(b.polygons[i].reference.at(other)),
                )
            })
            .collect()
    });
    Ok(aggregate(violations, Merge::AllDimensions))
}

/// Report point parts strictly inside polygon parts, with polygon origin provenance.
pub(super) fn contained_points(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    let a = t.parts(spec.layer)?;
    let b = t.parts(other)?;
    let locations = crate::engine::location::locations(a);
    let pairs = index::nearby(a.polygon_order(), b.point_index(), t.tolerance());
    let contained = |(i, j): (usize, usize)| {
        let (p, q) = (&a.polygons[i], &b.points[j]);
        t.cancellable(|| {
            locations[i].position(q.geometry.0) == 1
                && !locations[i].near(q.geometry.0, t.tolerance())
        })
        .then(|| {
            violation(
                spec,
                ViolationKind::Contained,
                q.geometry,
                Some(p.reference.at(spec.layer)),
                Some(q.reference.at(other)),
            )
        })
    };
    Ok(aggregate(
        super::each_pair(pairs, contained),
        Merge::AllDimensions,
    ))
}

/// Count point features whose retained parts all lie strictly inside a polygon feature.
pub(super) fn point_counts(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    count: PointCount,
) -> Result<Vec<Violation>, Error> {
    let av = cross::views(t.parts(spec.layer)?);
    let bv = cross::views(t.parts(other)?);
    let mut assignments = vec![0usize; bv.features.len()];
    let mut counts = vec![0; av.features.len()];
    for (i, j) in index::pairs(&av.feature_index, &bv.feature_index, 0., false) {
        let (polygon, point) = (&av.features[i], &bv.features[j]);
        if !point.points.is_empty()
            && point
                .points
                .iter()
                .all(|p| polygon.position(p.0) == 1 && !polygon.near(p.0, t.tolerance()))
        {
            assignments[j] += 1;
            counts[i] += 1;
        }
    }
    let (kind, valid): (_, fn(usize) -> bool) = match count {
        PointCount::AtLeastOne => (ViolationKind::MissingPoint, |n| n > 0),
        PointCount::ExactlyOne => (ViolationKind::PointCount, |n| n == 1),
    };
    let mut violations = Vec::new();
    for (i, p) in av.features.iter().enumerate() {
        if !valid(counts[i]) {
            violations.push(violation(
                spec,
                kind,
                super::feature_geometry(t, spec, spec.layer, p)?,
                Some(p.reference.at(spec.layer)),
                None,
            ));
        }
    }
    if matches!(count, PointCount::ExactlyOne) {
        for (j, p) in bv.features.iter().enumerate() {
            if assignments[j] != 1 {
                violations.push(violation(
                    spec,
                    ViolationKind::PointCount,
                    super::feature_geometry(t, spec, other, p)?,
                    None,
                    Some(p.reference.at(other)),
                ));
            }
        }
    }
    Ok(aggregate(violations, Merge::AllDimensions))
}
