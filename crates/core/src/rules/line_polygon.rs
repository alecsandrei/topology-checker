//! Line parts against individual polygon areas and their boundaries.
use super::{selected_pieces, violation};
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{
        cluster::distance,
        index,
        line_graph::{projection, proper_intersection},
        location::{self, Location},
        polygons,
    },
    model::Part,
};
use geo::{
    BoundingRect, Coord, Distance, Euclidean, Geometry, Length, Line, LineString, Point, Polygon,
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
    let union = polygons::union(t.parts(other)?);
    let violations = a
        .lines
        .par_iter()
        .filter_map(|l| {
            let inside = union
                .geometry
                .properly_contains(l.geometry.as_ref(), t.tolerance());
            (!inside).then(|| {
                violation(
                    spec,
                    ViolationKind::Outside,
                    l.geometry.as_ref().clone(),
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
    let bv = location::locations(b);
    let violations = index::pairs(a.line_index(), b.polygon_index(), t.tolerance(), false)
        .par_bridge()
        .flat_map_iter(|(i, j)| {
            t.cancellable(|| pair(t, spec, other, mode, &a.lines[i], &b.polygons[j], &bv[j]))
        })
        .collect();
    Ok(aggregate(violations, Merge::AllDimensions))
}

#[derive(Debug)]
struct Cut {
    position: f64,
    point: Coord,
}

/// Parameter intervals covered by a boundary segment's tolerance capsule.
/// Crossing evidence must leave this band, even at a short transverse contact.
fn boundary_band(segment: Line, boundary: Line, tolerance: f64) -> Vec<[f64; 2]> {
    let (vx, vy) = (
        segment.end.x - segment.start.x,
        segment.end.y - segment.start.y,
    );
    let length = vx.hypot(vy);
    let (bx, by) = (
        boundary.end.x - boundary.start.x,
        boundary.end.y - boundary.start.y,
    );
    let boundary_length = bx.hypot(by);
    let (ux, uy) = (bx / boundary_length, by / boundary_length);
    let (px, py) = (
        segment.start.x - boundary.start.x,
        segment.start.y - boundary.start.y,
    );
    let mut spans = Vec::with_capacity(3);
    let mut strip = [0., 1.];
    for (start, change, low, high) in [
        (px * ux + py * uy, vx * ux + vy * uy, 0., boundary_length),
        (px * uy - py * ux, vx * uy - vy * ux, -tolerance, tolerance),
    ] {
        if change == 0. {
            if start < low || start > high {
                strip = [1., 0.];
                break;
            }
        } else {
            let a = (low - start) / change;
            let b = (high - start) / change;
            strip[0] = f64::max(strip[0], a.min(b));
            strip[1] = f64::min(strip[1], a.max(b));
        }
    }
    if strip[0] < strip[1] {
        spans.push(strip);
    }
    for p in [boundary.start, boundary.end] {
        let (dx, dy) = (p.x - segment.start.x, p.y - segment.start.y);
        let perpendicular = (dx * vy - dy * vx) / length;
        if perpendicular.abs() <= tolerance {
            let center = (dx * (vx / length) + dy * (vy / length)) / length;
            let radius = ((tolerance - perpendicular.abs()) * (tolerance + perpendicular.abs()))
                .sqrt()
                / length;
            let span = [(center - radius).max(0.), (center + radius).min(1.)];
            if span[0] < span[1] {
                spans.push(span);
            }
        }
    }
    crate::engine::matching::union(spans)
}
fn pair(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
    mode: LinePolygon,
    line: &Part<crate::engine::storage::Shared<LineString>>,
    polygon: &Part<crate::engine::storage::Shared<Polygon>>,
    views: &Location,
) -> Vec<Violation> {
    let origin = Some(line.reference.at(spec.layer));
    let destination = Some(polygon.reference.at(other));
    let mut cuts = Vec::new();
    let mut contacts = Vec::new();
    let mut crossing_contacts = Vec::new();
    let mut boundary_intervals = Vec::new();
    for (i, segment) in line.geometry.lines().enumerate() {
        crate::cancel::check();
        cuts.push(Cut {
            position: i as f64,
            point: segment.start,
        });
        cuts.push(Cut {
            position: (i + 1) as f64,
            point: segment.end,
        });
        let mut candidates = views
            .tree
            .locate_in_envelope_intersecting(&index::expanded(
                segment.bounding_rect(),
                t.tolerance(),
            ))
            .map(|i| i.data)
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        for j in candidates {
            let boundary = views.segments[j];
            if let Some(intersection) = line_intersection(segment, boundary) {
                let points = match intersection {
                    LineIntersection::SinglePoint {
                        intersection,
                        is_proper,
                    } => vec![if is_proper {
                        proper_intersection(segment, boundary).unwrap_or(intersection)
                    } else {
                        intersection
                    }],
                    LineIntersection::Collinear { intersection } => {
                        let first = i as f64 + projection(segment, intersection.start).0;
                        let last = i as f64 + projection(segment, intersection.end).0;
                        boundary_intervals.push((first.min(last), first.max(last)));
                        vec![intersection.start, intersection.end]
                    }
                };
                for point in points {
                    let position = i as f64 + projection(segment, point).0;
                    cuts.push(Cut { position, point });
                    contacts.push(Point(point));
                    if mode == LinePolygon::Cross {
                        crossing_contacts.push((position, point));
                    }
                }
            }
            if mode == LinePolygon::Cross && t.tolerance() > 0. {
                for span in boundary_band(segment, boundary, t.tolerance()) {
                    boundary_intervals.push((i as f64 + span[0], i as f64 + span[1]));
                    for parameter in span {
                        cuts.push(Cut {
                            position: i as f64 + parameter,
                            point: crate::engine::matching::point(segment, parameter),
                        });
                    }
                }
            }
            if let Some([span, _]) =
                crate::engine::matching::interval(segment, boundary, t.tolerance())
            {
                let first = i as f64 + span[0];
                let last = i as f64 + span[1];
                boundary_intervals.push((first, last));
                for parameter in span {
                    cuts.push(Cut {
                        position: i as f64 + parameter,
                        point: crate::engine::matching::point(segment, parameter),
                    });
                }
            }
            if mode == LinePolygon::Intersect && t.tolerance() > 0. {
                // A minimum segment distance occurs at an endpoint projection unless
                // they cross, which has already supplied an exact contact.
                for point in [segment.start, segment.end] {
                    if Euclidean.distance(&Point(point), &boundary) <= t.tolerance() {
                        contacts.push(Point(point));
                    }
                }
                for point in [boundary.start, boundary.end] {
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
    let boundary_intervals = crate::engine::matching::union(
        boundary_intervals
            .into_iter()
            .map(|(a, b)| [a, b])
            .collect(),
    );
    let mut intervals = boundary_intervals.iter().peekable();
    let states = cuts
        .windows(2)
        .map(|pair| {
            while intervals
                .peek()
                .is_some_and(|span| span[1] <= pair[0].position)
            {
                intervals.next();
            }
            if intervals
                .peek()
                .is_some_and(|span| pair[0].position >= span[0] && pair[1].position <= span[1])
            {
                0
            } else if crate::engine::arrangement::midpoint_inside(
                views,
                pair[0].point,
                pair[1].point,
            ) {
                1
            } else {
                -1
            }
        })
        .collect::<Vec<_>>();
    if mode == LinePolygon::Cross {
        crossing_contacts.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let mut violations = Vec::new();
        let open = line.geometry.0.first() != line.geometry.0.last();
        // Boundary intervals join the two original passages on their sides.
        // A change between inside and outside is still a crossing when the
        // connecting passage follows the boundary. Closed paths also join at
        // their traversal seam, including a boundary run across that seam.
        let mut previous = (!open)
            .then(|| {
                states
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, state)| **state != 0)
                    .map(|(i, &state)| (i, state))
            })
            .flatten();
        for (i, &state) in states.iter().enumerate() {
            if state == 0 {
                continue;
            }
            let Some((before, prior)) = previous.replace((i, state)) else {
                continue;
            };
            if prior * state != -1 {
                continue;
            }
            // A boundary run has two possible transition endpoints. Select by
            // original coordinate order so reversal and ring rotation agree.
            let a = cuts[before + 1].point;
            let b = cuts[i].point;
            let coordinate_order =
                |a: &Coord, b: &Coord| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y));
            let start = crossing_contacts.partition_point(|&(position, _)| {
                #[cfg(test)]
                CROSSING_CONTACT_VISITS.with(|n| n.set(n.get() + 1));
                position < cuts[before + 1].position
            });
            let end = crossing_contacts.partition_point(|&(position, _)| {
                #[cfg(test)]
                CROSSING_CONTACT_VISITS.with(|n| n.set(n.get() + 1));
                position <= cuts[i].position
            });
            let (first, seam) = if before < i {
                (&crossing_contacts[start..end], &[][..])
            } else {
                (&crossing_contacts[start..], &crossing_contacts[..end])
            };
            let contact = first
                .iter()
                .chain(seam)
                .inspect(|_| {
                    #[cfg(test)]
                    CROSSING_CONTACT_VISITS.with(|n| n.set(n.get() + 1));
                })
                .map(|&(_, point)| point)
                .min_by(coordinate_order)
                .unwrap_or_else(|| {
                    if coordinate_order(&a, &b).is_le() {
                        a
                    } else {
                        b
                    }
                });
            let endpoint_contact = open
                && [
                    line.geometry.0[0],
                    *line.geometry.0.last().expect("clean line"),
                ]
                .into_iter()
                .any(|p| distance(p, contact) <= t.tolerance());
            if !endpoint_contact {
                violations.push(violation(
                    spec,
                    ViolationKind::Crossing,
                    Point(contact),
                    origin,
                    destination,
                ));
            }
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
            // Select by original coordinate order so reversal agrees.
            let first = piece
                .0
                .iter()
                .min_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)))
                .expect("piece has coordinates");
            contacts.push(Point(*first));
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

/// Split source segments against local boundaries, then classify covered intervals.
pub(super) fn inside(
    t: &Topology,
    spec: RuleSpec,
    other: LayerId,
) -> Result<Vec<Violation>, Error> {
    let input = t.parts(spec.layer)?;
    let target = t.parts(other)?;
    let target_locations = location::locations(target);
    let violations = input
        .lines
        .par_iter()
        .flat_map_iter(|line| {
            outside_pieces(line.geometry.as_ref(), target, target_locations)
                .into_iter()
                .filter(|piece| Euclidean.length(piece) > t.tolerance())
                .map(|piece| {
                    violation(
                        spec,
                        ViolationKind::Outside,
                        piece,
                        Some(line.reference.at(spec.layer)),
                        None,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    Ok(aggregate(violations, Merge::AllDimensions))
}

#[cfg(test)]
thread_local! { static SEGMENT_POLYGONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
thread_local! { static CROSSING_CONTACT_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn outside_pieces(
    line: &LineString,
    target: &crate::model::Parts,
    locations: &[Location],
) -> Vec<LineString> {
    let mut coords = Vec::new();
    let mut outside = Vec::new();
    for segment in line.lines() {
        crate::cancel::check();
        // Polygon bounds restrict both boundary cuts and interval classification.
        // A whole-line box can otherwise select every polygon along a long route.
        let mut candidates = target
            .polygon_index()
            .locate_in_envelope_intersecting(&index::envelope(segment.bounding_rect()))
            .map(|item| item.data)
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        #[cfg(test)]
        SEGMENT_POLYGONS.with(|n| n.set(n.get() + candidates.len()));
        let mut cuts = vec![
            Cut {
                position: 0.,
                point: segment.start,
            },
            Cut {
                position: 1.,
                point: segment.end,
            },
        ];
        let mut shared = Vec::new();
        for &i in &candidates {
            let location = &locations[i];
            for item in location
                .tree
                .locate_in_envelope_intersecting(&index::envelope(segment.bounding_rect()))
            {
                crate::cancel::check();
                let boundary = location.segments[item.data];
                if let Some(contact) = line_intersection(segment, boundary) {
                    let points = match contact {
                        LineIntersection::SinglePoint {
                            intersection,
                            is_proper,
                        } => vec![if is_proper {
                            proper_intersection(segment, boundary).unwrap_or(intersection)
                        } else {
                            intersection
                        }],
                        LineIntersection::Collinear { intersection } => {
                            let a = projection(segment, intersection.start).0;
                            let b = projection(segment, intersection.end).0;
                            shared.push((a.min(b), a.max(b)));
                            vec![intersection.start, intersection.end]
                        }
                    };
                    for point in points {
                        cuts.push(Cut {
                            position: projection(segment, point).0,
                            point,
                        });
                    }
                }
            }
        }
        cuts.sort_by(|a, b| a.position.total_cmp(&b.position));
        cuts.dedup_by(|a, b| a.position == b.position);
        outside.extend(cuts.windows(2).map(|pair| {
            !shared
                .iter()
                .any(|&(a, b)| pair[0].position >= a && pair[1].position <= b)
                && !candidates.iter().any(|&i| {
                    crate::engine::arrangement::midpoint_inside(
                        &locations[i],
                        pair[0].point,
                        pair[1].point,
                    )
                })
        }));
        if coords.is_empty() {
            coords.push(cuts[0].point);
        }
        coords.extend(cuts[1..].iter().map(|c| c.point));
    }
    selected_pieces(&coords, &outside)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crossing_contact_selection_visits_only_transition_ranges() {
        let count = 4096;
        for tolerance in [0., 0.001] {
            let mut t = Topology::new(tolerance).unwrap();
            let source = t.add_layer(crate::Layer::from_geometries(
                "zigzag",
                [LineString(
                    (0..count)
                        .map(|i| Coord {
                            x: if i % 2 == 0 { -5. } else { 5. },
                            y: 5. + i as f64 / count as f64,
                        })
                        .collect(),
                )
                .into()],
            ));
            let target = t.add_layer(crate::Layer::from_geometries(
                "box",
                [
                    geo::Rect::new(Coord { x: 0., y: 0. }, Coord { x: 10., y: 10. })
                        .to_polygon()
                        .into(),
                ],
            ));
            t.add_rule(
                crate::Rule::LineMustNotCrossPolygonBoundaries,
                source,
                Some(target),
            )
            .unwrap();
            let a = t.parts(source).unwrap();
            let b = t.parts(target).unwrap();
            CROSSING_CONTACT_VISITS.with(|n| n.set(0));
            let violations = pair(
                &t,
                t.rules().next().unwrap(),
                target,
                LinePolygon::Cross,
                &a.lines[0],
                &b.polygons[0],
                &location::locations(b)[0],
            );
            assert_eq!(violations.len(), count - 1);
            assert!(CROSSING_CONTACT_VISITS.with(|n| n.get()) <= 32 * count);
        }
    }
    #[test]
    fn line_coverage_queries_only_segment_local_polygons() {
        let mut t = Topology::new(0.001).unwrap();
        let target = t.add_layer(crate::Layer::from_geometries(
            "boxes",
            (0..40).map(|i| {
                let x = i as f64 * 25.;
                Geometry::Polygon(
                    geo::Rect::new(Coord { x, y: -5. }, Coord { x: x + 3., y: 5. }).to_polygon(),
                )
            }),
        ));
        let target = t.parts(target).unwrap();
        let line = LineString(
            (0..8000)
                .map(|i| {
                    let x = i as f64 / 8.;
                    Coord { x, y: x.sin() }
                })
                .collect(),
        );
        SEGMENT_POLYGONS.with(|n| n.set(0));
        let pieces = outside_pieces(&line, target, location::locations(target));
        assert_eq!(pieces.len(), 40);
        assert!(SEGMENT_POLYGONS.with(|n| n.get()) < 2000);
        assert!(pieces.iter().all(|p| p.0.len() > 2));
    }
}
