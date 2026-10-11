//! Line parts against individual polygon areas and their boundaries.
use super::{boundary_distance, selected_pieces, violation};
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{
        cluster::distance,
        cross::{self, Views},
        index,
        line_graph::{projection, proper_intersection},
        polygons,
    },
    model::Part,
};
use geo::{
    BoundingRect, Contains, Coord, Distance, Euclidean, Geometry, Length, LineString, Point,
    Polygon, Relate,
    line_intersection::{LineIntersection, line_intersection},
};
use rayon::prelude::*;

/// Which contact between a line part and a polygon part is a violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LinePolygon {
    /// Entry and exit crossings of an exterior or hole boundary.
    Cross,
    /// Pieces inside the area or along its boundary, longer than tolerance.
    Overlap,
    /// Any piece or isolated contact, with boundary proximity counting as contact.
    Intersect,
}

/// Report whole line parts outside the polygon union or within tolerance of its boundary.
pub(super) fn properly_inside(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    let a = t.parts(spec.layer)?;
    let union = &polygons::union(t.parts(other)?).geometry;
    let violations = a
        .lines
        .par_iter()
        .filter_map(|l| {
            let inside = union.relate(&l.geometry).is_contains()
                && union
                    .0
                    .iter()
                    .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
                    .all(|r| Euclidean.distance(&l.geometry, r) > t.tolerance());
            (!inside).then(|| {
                violation(
                    spec,
                    ViolationKind::Outside,
                    l.geometry.clone(),
                    Some(l.reference.at(spec.layer)),
                    None,
                )
            })
        })
        .collect();
    Ok(aggregate(violations, Merge::AllDimensions))
}

/// Compare every nearby line and polygon part pair.
pub(super) fn run(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    mode: LinePolygon,
) -> Result<Vec<Violation>, Error> {
    let a = t.parts(spec.layer)?;
    let b = t.parts(other)?;
    let bv = cross::views(b);
    let violations = index::pairs(&a.line_index, &b.polygon_index, t.tolerance(), false)
        .par_iter()
        .flat_map_iter(|&(i, j)| pair(t, spec, other, mode, &a.lines[i], &b.polygons[j], bv))
        .collect();
    Ok(aggregate(violations, Merge::AllDimensions))
}

#[derive(Debug)]
struct Cut {
    position: f64,
    point: Coord,
}
fn pair(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    mode: LinePolygon,
    line: &Part<LineString>,
    polygon: &Part<Polygon>,
    views: &Views,
) -> Vec<Violation> {
    let origin = Some(line.reference.at(spec.layer));
    let destination = Some(polygon.reference.at(other));
    let mut cuts = Vec::new();
    let mut contacts = Vec::new();
    for (i, segment) in line.geometry.lines().enumerate() {
        cuts.push(Cut {
            position: i as f64,
            point: segment.start,
        });
        cuts.push(Cut {
            position: (i + 1) as f64,
            point: segment.end,
        });
        let mut candidates = views
            .boundary_index
            .locate_in_envelope_intersecting(&index::expanded(
                segment.bounding_rect(),
                t.tolerance(),
            ))
            .map(|i| i.data)
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        for j in candidates {
            let boundary = &views.boundary_segments[j];
            // Both references come from the polygon layer's cached views.
            if (boundary.reference.fid, boundary.reference.part)
                != (polygon.reference.fid, polygon.reference.part)
            {
                continue;
            }
            if let Some(intersection) = line_intersection(segment, boundary.geometry) {
                let points = match intersection {
                    LineIntersection::SinglePoint {
                        intersection,
                        is_proper,
                    } => vec![if is_proper {
                        proper_intersection(segment, boundary.geometry).unwrap_or(intersection)
                    } else {
                        intersection
                    }],
                    LineIntersection::Collinear { intersection } => {
                        vec![intersection.start, intersection.end]
                    }
                };
                for point in points {
                    cuts.push(Cut {
                        position: i as f64 + projection(segment, point).0,
                        point,
                    });
                    contacts.push(Point(point));
                }
            }
            if mode == LinePolygon::Intersect && t.tolerance() > 0. {
                // A minimum segment distance occurs at an endpoint projection unless
                // they cross, which has already supplied an exact contact.
                for point in [segment.start, segment.end] {
                    if Euclidean.distance(&Point(point), &boundary.geometry) <= t.tolerance() {
                        contacts.push(Point(point));
                    }
                }
                for point in [boundary.geometry.start, boundary.geometry.end] {
                    let (_, projected) = projection(segment, point);
                    if distance(point, projected) <= t.tolerance() {
                        contacts.push(Point(projected));
                    }
                }
            }
        }
    }
    cuts.sort_by(|a, b| a.position.total_cmp(&b.position));
    cuts.dedup_by(|a, b| a.position == b.position);
    let states = cuts
        .windows(2)
        .map(|pair| {
            let midpoint = Point(Coord {
                x: pair[0].point.x + (pair[1].point.x - pair[0].point.x) / 2.,
                y: pair[0].point.y + (pair[1].point.y - pair[0].point.y) / 2.,
            });
            if boundary_distance(&polygon.geometry, &midpoint) == 0. {
                0
            } else if polygon.geometry.contains(&midpoint) {
                1
            } else {
                -1
            }
        })
        .collect::<Vec<_>>();
    if mode == LinePolygon::Cross {
        let mut violations = Vec::new();
        for i in 1..cuts.len().saturating_sub(1) {
            if states[i - 1] * states[i] == -1 {
                violations.push(violation(
                    spec,
                    ViolationKind::Crossing,
                    Point(cuts[i].point),
                    origin,
                    destination,
                ));
            }
        }
        if line.geometry.0.first() == line.geometry.0.last()
            && states.len() > 1
            && states[0] * states[states.len() - 1] == -1
        {
            violations.push(violation(
                spec,
                ViolationKind::Crossing,
                Point(cuts[0].point),
                origin,
                destination,
            ));
        }
        return violations;
    }
    let any = mode == LinePolygon::Intersect;
    let mut pieces = Vec::new();
    let coords = cuts.iter().map(|cut| cut.point).collect::<Vec<_>>();
    let selected = states.iter().map(|&state| state >= 0).collect::<Vec<_>>();
    for piece in selected_pieces(&coords, &selected) {
        if Euclidean.length(&piece) > t.tolerance() {
            pieces.push(piece);
        } else if any && piece.0.len() >= 2 {
            contacts.push(Point(piece.0[0]));
        }
    }
    let mut geometries = pieces
        .iter()
        .cloned()
        .map(Geometry::LineString)
        .collect::<Vec<_>>();
    if any {
        geometries.extend(
            contacts
                .into_iter()
                .filter(|p| !pieces.iter().any(|l| Euclidean.distance(l, p) == 0.))
                .map(Geometry::Point),
        );
    }
    if geometries.is_empty() {
        Vec::new()
    } else {
        vec![violation(
            spec,
            if any {
                ViolationKind::Intersection
            } else {
                ViolationKind::Overlap
            },
            Merge::AllDimensions.geometry(geometries),
            origin,
            destination,
        )]
    }
}
