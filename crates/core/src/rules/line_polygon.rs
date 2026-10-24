//! Line parts against individual polygon areas and their boundaries.
use super::{selected_pieces, selected_runs, violation};
use crate::{
    Error, LayerId, RuleSpec, Topology, Violation, ViolationKind,
    aggregate::{Merge, aggregate},
    engine::{
        cluster::distance,
        index,
        line_graph::{projection, proper_intersection},
        location::{self, Location},
        parameter::{self, Parameter},
        polygons, precision, proximity,
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
    let target = t.parts(other)?;
    let union = polygons::union(target);
    let violations = a
        .lines
        .par_iter()
        .filter_map(|l| {
            let inside = t.cancellable(|| {
                union.geometry.properly_contains(
                    l.geometry.as_ref(),
                    t.tolerance(),
                    &|segment, distance| {
                        boundary(segment, target, location::locations(target), distance)
                    },
                )
            });
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

#[derive(Debug, Clone, Copy)]
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
/// Cuts and window states for a line part against one polygon part.
struct Cuts {
    cuts: Vec<Cut>,
    states: Vec<i8>,
    contacts: Vec<Point>,
    crossing_contacts: Vec<(f64, Coord)>,
    exact: Option<ExactCuts>,
}
/// Zero-tolerance cut data: positions are exact ranks.
struct ExactCuts {
    /// The representative parameter of each cut rank.
    params: Vec<Parameter>,
    /// Cut rank of each contact.
    contact_ranks: Vec<usize>,
}
fn float_cuts(t: &Topology, mode: LinePolygon, line: &LineString, views: &Location) -> Cuts {
    let mut cuts = Vec::new();
    let mut contacts = Vec::new();
    let mut crossing_contacts = Vec::new();
    let mut boundary_intervals = Vec::new();
    for (i, segment) in line.lines().enumerate() {
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
    Cuts {
        cuts,
        states,
        contacts,
        crossing_contacts,
        exact: None,
    }
}
/// Zero tolerance: order cuts by exact segment parameters, deduplicate equal
/// parameters, and classify each window at its exact midpoint. Contacts are
/// identified by rank, so no decision uses a rounded coordinate. Line parts
/// have no repeated consecutive coordinates, so every segment, and every
/// window between distinct parameters, has positive length.
fn exact_cuts(mode: LinePolygon, line: &LineString, views: &Location) -> Cuts {
    let lines = line.lines().collect::<Vec<_>>();
    let mut params = Vec::with_capacity(lines.len() + 1);
    let mut contact_ids = Vec::new();
    let mut shared = Vec::new();
    let contact = |params: &mut Vec<Parameter>, ids: &mut Vec<usize>, p: Parameter| {
        ids.push(params.len());
        params.push(p.normalized(&lines));
    };
    for (i, &segment) in lines.iter().enumerate() {
        crate::cancel::check();
        params.push(Parameter::start(i, segment));
        if i + 1 == lines.len() {
            params.push(Parameter::end(i, segment));
        }
        let mut candidates = views
            .tree
            .locate_in_envelope_intersecting(&index::envelope(segment.bounding_rect()))
            .map(|i| i.data)
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        for j in candidates {
            let boundary = views.segments[j];
            match line_intersection(segment, boundary) {
                None => {}
                Some(LineIntersection::SinglePoint {
                    is_proper: true, ..
                }) => {
                    let p = Parameter::crossing(i, segment, boundary).expect("proper crossing");
                    contact(&mut params, &mut contact_ids, p);
                }
                Some(LineIntersection::SinglePoint { intersection, .. }) => {
                    let p = Parameter::on(i, segment, intersection);
                    contact(&mut params, &mut contact_ids, p);
                }
                Some(LineIntersection::Collinear { intersection }) => {
                    shared.push([params.len(), params.len() + 1]);
                    for p in [intersection.start, intersection.end] {
                        contact(&mut params, &mut contact_ids, Parameter::on(i, segment, p));
                    }
                }
            }
        }
    }
    let mut order = (0..params.len()).collect::<Vec<_>>();
    order.sort_by(|&a, &b| params[a].cmp(&params[b]));
    let mut rank = vec![0; params.len()];
    let mut representatives: Vec<usize> = Vec::new();
    for (k, &id) in order.iter().enumerate() {
        if k == 0 || params[order[k - 1]].cmp(&params[id]).is_ne() {
            representatives.push(id);
        }
        rank[id] = representatives.len() - 1;
    }
    // Windows between the ends of a collinear contact follow the boundary.
    let mut along = vec![0i32; representatives.len() + 1];
    for [a, b] in shared {
        let (a, b) = (rank[a].min(rank[b]), rank[a].max(rank[b]));
        along[a] += 1;
        along[b] -= 1;
    }
    let mut depth = 0;
    let states = representatives
        .windows(2)
        .enumerate()
        .map(|(r, pair)| {
            depth += along[r];
            if depth > 0 {
                0
            } else {
                parameter::window_position(views, &params[pair[0]], &params[pair[1]])
            }
        })
        .collect();
    let cuts = representatives
        .iter()
        .enumerate()
        .map(|(r, &id)| Cut {
            position: r as f64,
            point: params[id].point,
        })
        .collect::<Vec<_>>();
    let contact_ranks = contact_ids.iter().map(|&id| rank[id]).collect::<Vec<_>>();
    // Crossing mode reads only the ranked crossing contacts.
    let (contacts, crossing_contacts) = if mode == LinePolygon::Cross {
        (
            Vec::new(),
            contact_ranks
                .iter()
                .map(|&r| (r as f64, cuts[r].point))
                .collect(),
        )
    } else {
        (
            contact_ranks
                .iter()
                .map(|&r| Point(cuts[r].point))
                .collect(),
            Vec::new(),
        )
    };
    let mut slots = params.into_iter().map(Some).collect::<Vec<_>>();
    let params = representatives
        .iter()
        .map(|&id| slots[id].take().expect("one representative per parameter"))
        .collect();
    Cuts {
        cuts,
        states,
        contacts,
        crossing_contacts,
        exact: Some(ExactCuts {
            params,
            contact_ranks,
        }),
    }
}
impl ExactCuts {
    /// Emit one selected run of cut ranks. A crossing that is not
    /// representable lies inside a single boundary edge and changes sides, so
    /// it can only end a run; interior cuts are input coordinates. Run ends
    /// keep their nearest rounding, so a crossing shared by adjacent pieces
    /// has one coordinate. Only an end that rounds onto its neighbour is
    /// rounded away from the run instead (`Parameter::outward`), which keeps
    /// the piece within its segments and never zero-length.
    fn emit(&self, run: &[usize]) -> LineString {
        let last = run.len() - 1;
        let mut coords = run
            .iter()
            .map(|&c| self.params[c].point)
            .collect::<Vec<_>>();
        if coords[0] == coords[1] {
            coords[0] = self.params[run[0]].outward(false);
        }
        if coords[last] == coords[last - 1] {
            coords[last] = self.params[run[last]].outward(true);
        }
        // In a two-point run the end may have moved onto the start.
        if coords[0] == coords[1] {
            coords[0] = self.params[run[0]].outward(false);
        }
        // Exact parameters along a segment are monotone on each axis, input
        // coordinates at distinct parameters differ, and an outward rounding
        // moves away from its neighbour on an axis along which the segment
        // advances.
        debug_assert!(
            coords.windows(2).all(|w| w[0] != w[1]),
            "collapsed run {coords:?}"
        );
        LineString(coords)
    }
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
    let Cuts {
        cuts,
        states,
        mut contacts,
        mut crossing_contacts,
        exact,
    } = if t.tolerance() == 0. {
        exact_cuts(mode, line.geometry.as_ref(), views)
    } else {
        float_cuts(t, mode, line.geometry.as_ref(), views)
    };
    if mode == LinePolygon::Cross {
        crossing_contacts.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let mut violations = Vec::new();
        let coords = &line.geometry.0;
        let open = coords.first() != coords.last();
        // Passages leave the boundary band. Band parameters are rounded relative
        // to each segment start, so an original vertex just beyond the tolerance
        // can lose its window. Such a vertex is still a passage.
        let mut windows = cuts
            .windows(2)
            .zip(&states)
            .filter(|(_, state)| **state != 0)
            .map(|(pair, &state)| (pair[0], pair[1], state))
            .peekable();
        let mut passages = Vec::new();
        if t.tolerance() > 0. {
            let vertices = &coords[..coords.len() - usize::from(!open)];
            for (k, &point) in vertices.iter().enumerate() {
                let position = k as f64;
                while let Some(window) = windows.next_if(|w| w.1.position <= position) {
                    passages.push(window);
                }
                if !views.near(point, t.tolerance()) {
                    let cut = Cut { position, point };
                    passages.push((cut, cut, views.position(point)));
                }
            }
        }
        passages.extend(windows);
        // Boundary intervals join the two original passages on their sides.
        // A change between inside and outside is still a crossing when the
        // connecting passage follows the boundary. Closed paths also join at
        // their traversal seam, including a boundary run across that seam.
        let mut previous = (!open).then(|| passages.len().checked_sub(1)).flatten();
        for (i, &(entry, _, state)) in passages.iter().enumerate() {
            let Some(before) = previous.replace(i) else {
                continue;
            };
            let (_, exit, prior) = passages[before];
            if prior * state != -1 {
                continue;
            }
            // A boundary run has two possible transition endpoints. Select by
            // original coordinate order so reversal and ring rotation agree.
            let a = exit.point;
            let b = entry.point;
            let coordinate_order =
                |a: &Coord, b: &Coord| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y));
            let start = crossing_contacts.partition_point(|&(position, _)| {
                #[cfg(test)]
                CROSSING_CONTACT_VISITS.with(|n| n.set(n.get() + 1));
                position < exit.position
            });
            let end = crossing_contacts.partition_point(|&(position, _)| {
                #[cfg(test)]
                CROSSING_CONTACT_VISITS.with(|n| n.set(n.get() + 1));
                position <= entry.position
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
    if let Some(exact) = &exact {
        // Each window joins two exactly distinct parameters, so every selected
        // run has positive length. Contacts on a run are covered by rank.
        let mut covered = vec![false; cuts.len()];
        for run in selected_runs(&coords, &selected) {
            for &c in &run {
                covered[c] = true;
            }
            pieces.push(exact.emit(&run));
        }
        contacts = contacts
            .into_iter()
            .zip(&exact.contact_ranks)
            .filter(|(_, rank)| !covered[**rank])
            .map(|(p, _)| p)
            .collect();
    } else {
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
            t.cancellable(|| {
                outside_pieces(
                    line.geometry.as_ref(),
                    target,
                    target_locations,
                    t.tolerance(),
                )
            })
            .into_iter()
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

/// Pieces outside every target polygon and farther than tolerance from every
/// original target boundary. Capsule cuts stay on the original source segment.
fn outside_pieces(
    line: &LineString,
    target: &crate::model::Parts,
    locations: &[Location],
    tolerance: f64,
) -> Vec<LineString> {
    let mut coords = Vec::new();
    let mut states = Vec::new();
    let (mut order, mut rank, mut representatives, mut along) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for segment in line.lines() {
        crate::cancel::check();
        // Polygon bounds restrict both boundary cuts and interval classification.
        // A whole-line box can otherwise select every polygon along a long route.
        // Expand outwards so binary rounding cannot exclude an exactly
        // coincident candidate before the certified distance test.
        let envelope = if tolerance > 0. {
            let bounds = segment.bounding_rect();
            rstar::AABB::from_corners(
                [
                    (bounds.min().x - tolerance).next_down(),
                    (bounds.min().y - tolerance).next_down(),
                ],
                [
                    (bounds.max().x + tolerance).next_up(),
                    (bounds.max().y + tolerance).next_up(),
                ],
            )
        } else {
            index::envelope(segment.bounding_rect())
        };
        let mut candidates = target
            .polygon_index()
            .locate_in_envelope_intersecting(&envelope)
            .map(|item| item.data)
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        #[cfg(test)]
        SEGMENT_POLYGONS.with(|n| n.set(n.get() + candidates.len()));
        let mut params = vec![Parameter::start(0, segment), Parameter::end(0, segment)];
        let mut shared = Vec::new();
        let mut covered = Vec::new();
        for &i in &candidates {
            let location = &locations[i];
            for item in location.tree.locate_in_envelope_intersecting(&envelope) {
                crate::cancel::check();
                let boundary = location.segments[item.data];
                if tolerance > 0. {
                    covered.extend(proximity::capsule(segment, boundary, tolerance));
                }
                match line_intersection(segment, boundary) {
                    None => {}
                    Some(LineIntersection::SinglePoint {
                        is_proper: true, ..
                    }) => params
                        .push(Parameter::crossing(0, segment, boundary).expect("proper crossing")),
                    Some(LineIntersection::SinglePoint { intersection, .. }) => {
                        params.push(Parameter::on(0, segment, intersection));
                    }
                    Some(LineIntersection::Collinear { intersection }) => {
                        shared.push([params.len(), params.len() + 1]);
                        for p in [intersection.start, intersection.end] {
                            params.push(Parameter::on(0, segment, p));
                        }
                    }
                }
            }
        }
        let covered = proximity::union(covered);
        // Order the cuts by exact parameter and classify each window at its
        // exact midpoint, so rounded cut coordinates never decide a side.
        order.clear();
        order.extend(0..params.len());
        order.sort_by(|&a, &b| params[a].cmp(&params[b]));
        rank.clear();
        rank.resize(params.len(), 0);
        representatives.clear();
        for (k, &id) in order.iter().enumerate() {
            if k == 0 || params[order[k - 1]].cmp(&params[id]).is_ne() {
                representatives.push(id);
            }
            rank[id] = representatives.len() - 1;
        }
        // Windows between the ends of a collinear contact follow the boundary.
        along.clear();
        along.resize(representatives.len() + 1, 0i32);
        for [a, b] in shared {
            let (a, b) = (rank[a].min(rank[b]), rank[a].max(rank[b]));
            along[a] += 1;
            along[b] -= 1;
        }
        let append = |coords: &mut Vec<Coord>, states: &mut Vec<bool>, a, b, outside| {
            if coords.is_empty() {
                coords.push(a);
            }
            if coords.last() != Some(&a) {
                coords.push(a);
                states.push(false);
            }
            if coords.last() != Some(&b) {
                coords.push(b);
                states.push(outside);
            }
        };
        let mut depth = 0;
        for (r, pair) in representatives.windows(2).enumerate() {
            depth += along[r];
            let (a, b) = (&params[pair[0]], &params[pair[1]]);
            let window = Line::new(a.point, b.point);
            // Cuts that round onto one coordinate enclose no length.
            if window.start == window.end && tolerance == 0. {
                continue;
            }
            let uncovered = depth <= 0
                && !candidates
                    .iter()
                    .any(|&i| parameter::window_position(&locations[i], a, b) >= 0);
            if !uncovered || covered.is_empty() {
                let (start, end) = if uncovered && tolerance > 0. {
                    (a.outward(false), b.outward(true))
                } else {
                    (window.start, window.end)
                };
                append(&mut coords, &mut states, start, end, uncovered);
            } else {
                for [start, end] in
                    proximity::outside(a.exact().clone(), b.exact().clone(), &covered)
                {
                    append(
                        &mut coords,
                        &mut states,
                        start.outward(segment, false),
                        end.outward(segment, true),
                        true,
                    );
                }
                // Retain covered portions and input vertices between residual
                // runs, without adding any artificial semantic joins.
                if coords.last() != Some(&window.end) {
                    let start = coords.last().copied().unwrap_or(window.start);
                    append(&mut coords, &mut states, start, window.end, false);
                }
            }
        }
    }
    selected_runs(&coords, &states)
        .into_iter()
        .map(|run| LineString(run.into_iter().map(|i| coords[i]).collect()))
        .collect()
}
/// Target boundary segments within `distance` of `segment`, directed with the
/// covered side on their left, each with the far end of the next segment.
fn boundary(
    segment: Line,
    target: &crate::model::Parts,
    locations: &[Location],
    distance: f64,
) -> Vec<(Line, Coord)> {
    let query = index::expanded(segment.bounding_rect(), distance);
    let mut edges = Vec::new();
    for item in target
        .polygon_index()
        .locate_in_envelope_intersecting(&query)
    {
        let location = &locations[item.data];
        for s in location.tree.locate_in_envelope_intersecting(&query) {
            let (edge, next) = location.edge(s.data);
            if precision::squared_gap(edge, segment) <= distance * distance {
                edges.push((edge, next));
            }
        }
    }
    edges
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
        let pieces = outside_pieces(&line, target, location::locations(target), 0.001);
        assert_eq!(pieces.len(), 40);
        assert!(SEGMENT_POLYGONS.with(|n| n.get()) < 2000);
        assert!(pieces.iter().all(|p| p.0.len() > 2));
    }
}
