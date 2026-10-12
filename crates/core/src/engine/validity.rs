//! Indexed validity checks after degenerate-ring cleanup.
use super::location::Location;
use crate::engine::index;
use geo::{
    BoundingRect, LineIntersection, LineString, Polygon, line_intersection::line_intersection,
};

pub(crate) fn ring(ring: &LineString) -> Option<&'static str> {
    let segments = ring.lines().collect::<Vec<_>>();
    let tree = index::build(
        segments
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.bounding_rect())),
    );
    for (i, segment) in segments.iter().enumerate() {
        crate::cancel::check();
        for item in tree.locate_in_envelope_intersecting(&index::envelope(segment.bounding_rect()))
        {
            let j = item.data;
            if j <= i {
                continue;
            }
            if let Some(contact) = line_intersection(*segment, segments[j]) {
                let adjacent = j == i + 1 || i == 0 && j == segments.len() - 1;
                if !adjacent || matches!(contact, LineIntersection::Collinear { .. }) {
                    return Some("self-intersection or closing spike");
                }
            }
        }
    }
    None
}

pub(crate) fn polygon(polygon: &Polygon) -> Option<(usize, &'static str)> {
    for (i, r) in std::iter::once(polygon.exterior())
        .chain(polygon.interiors())
        .enumerate()
    {
        if let Some(reason) = ring(r) {
            return Some((i, reason));
        }
    }
    if polygon.interiors().is_empty() {
        return None;
    }
    let rings = std::iter::once(polygon.exterior())
        .chain(polygon.interiors())
        .collect::<Vec<_>>();
    let areas = rings
        .iter()
        .map(|r| Polygon::new((*r).clone(), vec![]))
        .collect::<Vec<_>>();
    let locations = areas
        .iter()
        .map(super::location::Location::new)
        .collect::<Vec<_>>();
    let boxes = index::build(
        areas
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.bounding_rect().map(|r| (i, r))),
    );
    let mut parents = (0..rings.len()).collect::<Vec<_>>();
    let mut contact_nodes = std::collections::BTreeMap::new();
    let mut contact_uses = std::collections::BTreeSet::new();
    fn root(parents: &[usize], mut i: usize) -> usize {
        while parents[i] != i {
            i = parents[i];
        }
        i
    }
    for i in 1..rings.len() {
        if representative_position(&locations[i], &locations[0]) < 0 {
            return Some((i, "hole outside shell or crossing shell"));
        }
        for item in boxes.locate_in_envelope_intersecting(&index::envelope(
            areas[i].bounding_rect().expect("ring bounds"),
        )) {
            let j = item.data;
            if j >= i {
                continue;
            }
            let mut contacts = std::collections::BTreeSet::new();
            let i_is_small = locations[i].segments.len() <= locations[j].segments.len();
            let (small, large) = if i_is_small {
                (&locations[i], &locations[j])
            } else {
                (&locations[j], &locations[i])
            };
            for a in &small.segments {
                crate::cancel::check();
                let mut cuts = Vec::new();
                for item in large
                    .tree
                    .locate_in_envelope_intersecting(&index::envelope(a.bounding_rect()))
                {
                    if let Some(contact) = line_intersection(*a, large.segments[item.data]) {
                        match contact {
                            LineIntersection::Collinear { .. }
                            | LineIntersection::SinglePoint {
                                is_proper: true, ..
                            } => return Some((i, "crossing or overlapping polygon rings")),
                            LineIntersection::SinglePoint { intersection, .. } => {
                                cuts.push(intersection);
                                contacts.insert((
                                    crate::engine::cluster::bits(intersection.x),
                                    crate::engine::cluster::bits(intersection.y),
                                ));
                            }
                        }
                    }
                }
                if !cuts.is_empty() {
                    cuts.extend([a.start, a.end]);
                    cuts.sort_by(|p, q| {
                        super::line_graph::projection(*a, *p)
                            .0
                            .total_cmp(&super::line_graph::projection(*a, *q).0)
                    });
                    cuts.dedup();
                    if cuts.windows(2).any(|pair| {
                        let side = super::arrangement::midpoint_position(large, pair[0], pair[1]);
                        if j == 0 && i_is_small {
                            side < 0
                        } else {
                            side > 0
                        }
                    }) {
                        return Some((i, "crossing or overlapping polygon rings"));
                    }
                }
            }
            if j > 0
                && (representative_position(&locations[i], &locations[j]) == 1
                    || representative_position(&locations[j], &locations[i]) == 1)
            {
                return Some((i, "nested or overlapping holes"));
            }
            for contact in contacts {
                let node = *contact_nodes.entry(contact).or_insert_with(|| {
                    let id = parents.len();
                    parents.push(id);
                    id
                });
                // Contact locations are graph vertices too. Several holes
                // meeting at one articulation form a star, not a cycle.
                for ring in [i, j] {
                    if !contact_uses.insert((ring, node)) {
                        continue;
                    }
                    let a = root(&parents, ring);
                    let b = root(&parents, node);
                    if a == b {
                        return Some((i, "disconnected interior"));
                    }
                    parents[a] = b;
                }
            }
        }
    }
    None
}

// After indexed contact checks, a connected ring has one containment state.
// An inscribed ring can have every original vertex on the other boundary, so
// exact source-edge midpoints provide the fallback representative.
fn representative_position(a: &Location, b: &Location) -> i8 {
    for segment in &a.segments {
        crate::cancel::check();
        #[cfg(test)]
        CLASSIFIED_VERTICES.with(|n| n.set(n.get() + 1));
        let position = b.position(segment.start);
        if position != 0 {
            return position;
        }
    }
    a.segments
        .iter()
        .map(|s| super::arrangement::midpoint_position(b, s.start, s.end))
        .find(|&p| p != 0)
        .unwrap_or(0)
}

/// Reuse original boundary and ray indexes, scanning the smaller boundary only.
/// With crossings and shared edges excluded, a boundary component has a single
/// inside/outside state except at isolated contacts. Midpoints distinguish a
/// crossing through a vertex from a permitted point tangency.
pub(crate) fn multipart(a: &Location, b: &Location) -> bool {
    relation(a, b, true)
}
/// Positive-area contribution, using the same original boundary evidence as
/// multipart validation. Shared edges count only when their interior sides agree.
pub(crate) fn interiors_overlap(a: &Location, b: &Location) -> bool {
    relation(a, b, false)
}
/// Certify a contained original polygon without constructing another overlay.
/// A container with holes stays on the general overlay path.
pub(crate) fn covered_by(a: &Location, b: &Location) -> bool {
    if b.anchors.len() != 1
        || a.bounds.min().x < b.bounds.min().x
        || a.bounds.min().y < b.bounds.min().y
        || a.bounds.max().x > b.bounds.max().x
        || a.bounds.max().y > b.bounds.max().y
    {
        return false;
    }
    for segment in &a.segments {
        crate::cancel::check();
        if b.position(segment.start) < 0 {
            return false;
        }
        let mut cuts = Vec::new();
        for item in b
            .tree
            .locate_in_envelope_intersecting(&index::envelope(segment.bounding_rect()))
        {
            match line_intersection(*segment, b.segments[item.data]) {
                Some(LineIntersection::SinglePoint {
                    is_proper: true, ..
                }) => return false,
                Some(LineIntersection::SinglePoint { intersection, .. }) => cuts.push(intersection),
                Some(LineIntersection::Collinear { intersection }) => {
                    cuts.extend([intersection.start, intersection.end])
                }
                None => {}
            }
        }
        if !cuts.is_empty() {
            cuts.extend([segment.start, segment.end]);
            cuts.sort_by(|a, b| {
                super::line_graph::projection(*segment, *a)
                    .0
                    .total_cmp(&super::line_graph::projection(*segment, *b).0)
            });
            cuts.dedup();
            if cuts
                .windows(2)
                .any(|p| super::arrangement::midpoint_position(b, p[0], p[1]) < 0)
            {
                return false;
            }
        }
    }
    true
}
fn relation(a: &Location, b: &Location, reject_shared: bool) -> bool {
    let (small, large) = if a.segments.len() <= b.segments.len() {
        (a, b)
    } else {
        (b, a)
    };
    for (i, segment) in small.segments.iter().enumerate() {
        crate::cancel::check();
        #[cfg(test)]
        SCANNED.with(|n| n.set(n.get() + 1));
        let mut cuts = Vec::new();
        for item in large
            .tree
            .locate_in_envelope_intersecting(&index::envelope(segment.bounding_rect()))
        {
            match line_intersection(*segment, large.segments[item.data]) {
                Some(LineIntersection::Collinear { .. }) => {
                    let other = large.segments[item.data];
                    let same_direction = if segment.start.x != segment.end.x {
                        (segment.end.x > segment.start.x) == (other.end.x > other.start.x)
                    } else {
                        (segment.end.y > segment.start.y) == (other.end.y > other.start.y)
                    };
                    if reject_shared
                        || (small.interior_left[i] == large.interior_left[item.data])
                            == same_direction
                    {
                        return true;
                    }
                }
                Some(LineIntersection::SinglePoint {
                    is_proper: true, ..
                }) => return true,
                Some(LineIntersection::SinglePoint { intersection, .. }) => cuts.push(intersection),
                None => {}
            }
        }
        if !cuts.is_empty() {
            cuts.extend([segment.start, segment.end]);
            cuts.sort_by(|a, b| {
                super::line_graph::projection(*segment, *a)
                    .0
                    .total_cmp(&super::line_graph::projection(*segment, *b).0)
            });
            cuts.dedup();
            if cuts
                .windows(2)
                .any(|p| super::arrangement::midpoint_inside(large, p[0], p[1]))
            {
                return true;
            }
        }
    }
    // Each exterior and hole boundary is connected. One non-boundary vertex
    // decides containment after the robust contact checks above.
    fn contained(a: &Location, b: &Location) -> bool {
        a.anchors.iter().any(|&range| {
            a.segments[range.0..range.1]
                .iter()
                .map(|s| b.position(s.start))
                .find(|&p| p != 0)
                == Some(1)
        })
    }
    contained(a, b) || contained(b, a)
}

#[cfg(test)]
thread_local! { static SCANNED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
thread_local! { static CLASSIFIED_VERTICES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hole_containment_classifies_one_representative_after_contact_checks() {
        let shell =
            geo::Rect::new(geo::Coord { x: -2., y: -2. }, geo::Coord { x: 2., y: 2. }).to_polygon();
        let mut ring = (0..4096)
            .map(|i| {
                let t = i as f64 * std::f64::consts::TAU / 4096.;
                geo::Coord {
                    x: t.cos(),
                    y: t.sin(),
                }
            })
            .collect::<Vec<_>>();
        ring.push(ring[0]);
        CLASSIFIED_VERTICES.with(|n| n.set(0));
        assert!(
            polygon(&Polygon::new(
                shell.exterior().clone(),
                vec![LineString(ring)]
            ))
            .is_none()
        );
        assert_eq!(CLASSIFIED_VERTICES.with(|n| n.get()), 1);
        let inscribed = LineString::from(vec![(-2., 0.), (0., 2.), (2., 0.), (-2., 0.)]);
        assert!(polygon(&Polygon::new(shell.exterior().clone(), vec![inscribed])).is_some());
        let contact = LineString::from(vec![(-2., 0.), (-1., 1.), (1., 0.), (-2., 0.)]);
        assert!(polygon(&Polygon::new(shell.exterior().clone(), vec![contact])).is_none());
    }
    #[test]
    fn holes_at_one_contact_form_a_star_and_two_contacts_form_a_cycle() {
        let shell =
            geo::Rect::new(geo::Coord { x: 0., y: 0. }, geo::Coord { x: 10., y: 10. }).to_polygon();
        let holes = vec![
            LineString::from(vec![(5., 5.), (3., 4.), (3., 6.), (5., 5.)]),
            LineString::from(vec![(5., 5.), (4., 3.), (6., 3.), (5., 5.)]),
            LineString::from(vec![(5., 5.), (7., 6.), (7., 4.), (5., 5.)]),
        ];
        assert!(polygon(&Polygon::new(shell.exterior().clone(), holes)).is_none());
        let pinched = LineString::from(vec![(0., 4.), (4., 4.), (4., 6.), (0., 6.), (0., 4.)]);
        assert!(polygon(&Polygon::new(shell.exterior().clone(), vec![pinched])).is_some());
    }
    #[test]
    fn islands_query_small_boundaries_without_rescanning_large_rings() {
        let n = 20_000;
        let mut ring = (0..n)
            .map(|i| {
                let t = i as f64 * std::f64::consts::TAU / n as f64;
                geo::Coord {
                    x: 1000. * t.cos(),
                    y: 1000. * t.sin(),
                }
            })
            .collect::<Vec<_>>();
        ring.push(ring[0]);
        let disc = Location::new(&Polygon::new(LineString(ring), vec![]));
        SCANNED.with(|n| n.set(0));
        for i in 0..100 {
            let x = 800. + (i % 10) as f64 * 9.;
            let y = 800. + (i / 10) as f64 * 9.;
            let square = geo::Rect::new(
                geo::Coord { x, y },
                geo::Coord {
                    x: x + 4.,
                    y: y + 4.,
                },
            )
            .to_polygon();
            let island = Location::new(&square);
            assert!(!multipart(&disc, &island));
            assert!(!multipart(&island, &disc));
        }
        assert_eq!(SCANNED.with(|n| n.get()), 800);
    }
    #[test]
    fn crossing_through_vertices_is_not_a_point_contact() {
        let square =
            geo::Rect::new(geo::Coord { x: 0., y: 0. }, geo::Coord { x: 2., y: 2. }).to_polygon();
        let triangle = Polygon::new(
            LineString::from(vec![(-1., -1.), (3., 3.), (-1., 3.), (-1., -1.)]),
            vec![],
        );
        assert!(multipart(
            &Location::new(&square),
            &Location::new(&triangle)
        ));
    }
}
