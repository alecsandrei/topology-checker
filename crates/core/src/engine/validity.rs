//! Indexed validity checks after degenerate-ring cleanup.
use crate::engine::index;
use geo::{
    BoundingRect, LineIntersection, LineString, Polygon, Relate,
    line_intersection::line_intersection,
};
use geo::{coordinate_position::CoordPos, dimensions::Dimensions};

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
    fn root(parents: &[usize], mut i: usize) -> usize {
        while parents[i] != i {
            i = parents[i];
        }
        i
    }
    for i in 1..rings.len() {
        if rings[i].0.iter().any(|&p| locations[0].position(p) < 0) {
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
            for a in &locations[i].segments {
                for item in locations[j]
                    .tree
                    .locate_in_envelope_intersecting(&index::envelope(a.bounding_rect()))
                {
                    if let Some(contact) = line_intersection(*a, locations[j].segments[item.data]) {
                        match contact {
                            LineIntersection::Collinear { .. }
                            | LineIntersection::SinglePoint {
                                is_proper: true, ..
                            } => return Some((i, "crossing or overlapping polygon rings")),
                            LineIntersection::SinglePoint { intersection, .. } => {
                                contacts.insert((
                                    crate::engine::cluster::bits(intersection.x),
                                    crate::engine::cluster::bits(intersection.y),
                                ));
                            }
                        }
                    }
                }
            }
            if j > 0
                && (rings[i].0.iter().any(|&p| locations[j].position(p) == 1)
                    || rings[j].0.iter().any(|&p| locations[i].position(p) == 1))
            {
                return Some((i, "nested or overlapping holes"));
            }
            for _ in contacts {
                let a = root(&parents, i);
                let b = root(&parents, j);
                if a == b {
                    return Some((i, "disconnected interior"));
                }
                parents[a] = b;
            }
        }
    }
    None
}

pub(crate) fn multipart(a: &Polygon, b: &Polygon) -> bool {
    let matrix = a.relate(b);
    matrix.get(CoordPos::Inside, CoordPos::Inside) != Dimensions::Empty
        || matrix.get(CoordPos::OnBoundary, CoordPos::OnBoundary) == Dimensions::OneDimensional
}
