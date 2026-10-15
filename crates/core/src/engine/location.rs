//! Reusable boundary and ray indexes over original polygon coordinates.
use super::{
    index::{self, Index},
    proximity,
};
use geo::{
    BoundingRect, Coord, Kernel, Line, Polygon, Rect, Winding,
    kernels::{Orientation, RobustKernel},
};

#[derive(Debug)]
pub(crate) struct Location {
    pub segments: Vec<Line>,
    pub tree: Index,
    pub bounds: Rect,
    pub anchors: Vec<(usize, usize)>,
    pub interior_left: Vec<bool>,
}
impl Location {
    pub fn new(polygon: &Polygon) -> Self {
        let interior_left = std::iter::once(polygon.exterior())
            .chain(polygon.interiors())
            .enumerate()
            .flat_map(|(i, r)| {
                let left = (r.winding_order()
                    == Some(geo::winding_order::WindingOrder::CounterClockwise))
                    == (i == 0);
                std::iter::repeat_n(left, r.0.len() - 1)
            })
            .collect();
        let mut start = 0;
        let anchors = std::iter::once(polygon.exterior())
            .chain(polygon.interiors())
            .map(|ring| {
                let end = start + ring.0.len() - 1;
                let range = (start, end);
                start = end;
                range
            })
            .collect();
        let segments = std::iter::once(polygon.exterior())
            .chain(polygon.interiors())
            .flat_map(|r| r.lines())
            .collect::<Vec<_>>();
        let tree = index::build(
            segments
                .iter()
                .enumerate()
                .map(|(i, s)| (i, s.bounding_rect())),
        );
        Self {
            segments,
            tree,
            bounds: polygon.bounding_rect().expect("clean polygon"),
            anchors,
            interior_left,
        }
    }
    /// -1 outside, 0 boundary, 1 interior. Ray decisions use robust orientations.
    pub fn position(&self, p: Coord) -> i8 {
        if p.x < self.bounds.min().x
            || p.x > self.bounds.max().x
            || p.y < self.bounds.min().y
            || p.y > self.bounds.max().y
        {
            return -1;
        }
        let query = index::envelope(Rect::new(
            p,
            Coord {
                x: self.bounds.max().x,
                y: p.y,
            },
        ));
        let mut inside = false;
        for item in self.tree.locate_in_envelope_intersecting(&query) {
            let s = self.segments[item.data];
            let side = RobustKernel::orient2d(s.start, s.end, p);
            if side == Orientation::Collinear
                && p.x >= s.start.x.min(s.end.x)
                && p.x <= s.start.x.max(s.end.x)
                && p.y >= s.start.y.min(s.end.y)
                && p.y <= s.start.y.max(s.end.y)
            {
                return 0;
            }
            if (s.start.y > p.y) != (s.end.y > p.y)
                && (side == Orientation::CounterClockwise) == (s.end.y > s.start.y)
            {
                inside = !inside;
            }
        }
        if inside { 1 } else { -1 }
    }
    /// Segment `i` directed with the interior on its left, and the far end of
    /// the segment that follows it in that direction.
    pub fn edge(&self, i: usize) -> (Line, Coord) {
        let (start, end) = self.anchors[self.anchors.partition_point(|&(s, _)| s <= i) - 1];
        let s = self.segments[i];
        if self.interior_left[i] {
            (
                s,
                self.segments[if i + 1 == end { start } else { i + 1 }].end,
            )
        } else {
            (
                Line::new(s.end, s.start),
                self.segments[if i == start { end - 1 } else { i - 1 }].start,
            )
        }
    }
    pub fn near(&self, p: Coord, tolerance: f64) -> bool {
        self.tree
            .locate_in_envelope_intersecting(&index::at(p, tolerance))
            .any(|item| {
                let s = self.segments[item.data];
                if tolerance == 0. {
                    RobustKernel::orient2d(s.start, s.end, p) == Orientation::Collinear
                        && s.bounding_rect().contains_coord(p)
                } else {
                    proximity::near_segment(p, s, tolerance)
                }
            })
    }
}
trait ContainsCoordinate {
    fn contains_coord(&self, p: Coord) -> bool;
}
impl ContainsCoordinate for Rect {
    fn contains_coord(&self, p: Coord) -> bool {
        p.x >= self.min().x && p.x <= self.max().x && p.y >= self.min().y && p.y <= self.max().y
    }
}
pub(crate) fn locations(parts: &crate::model::Parts) -> &[Location] {
    parts.locations.get_or_init(|| {
        parts
            .polygons
            .iter()
            .map(|p| Location::new(&p.geometry))
            .collect()
    })
}
