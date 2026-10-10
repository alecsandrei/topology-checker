use geo::{Coord, Rect};
use rstar::{
    AABB, RTree,
    primitives::{GeomWithData, Rectangle},
};

pub(crate) type Index = RTree<GeomWithData<Rectangle<[f64; 2]>, usize>>;
pub(crate) fn envelope(rect: Rect) -> AABB<[f64; 2]> {
    AABB::from_corners([rect.min().x, rect.min().y], [rect.max().x, rect.max().y])
}
pub(crate) fn expanded(rect: Rect, tolerance: f64) -> AABB<[f64; 2]> {
    AABB::from_corners(
        [rect.min().x - tolerance, rect.min().y - tolerance],
        [rect.max().x + tolerance, rect.max().y + tolerance],
    )
}
pub(crate) fn at(point: Coord, tolerance: f64) -> AABB<[f64; 2]> {
    expanded(Rect::new(point, point), tolerance)
}
pub(crate) fn build(items: impl IntoIterator<Item = (usize, Rect)>) -> Index {
    RTree::bulk_load(
        items
            .into_iter()
            .map(|(i, r)| GeomWithData::new(Rectangle::from_aabb(envelope(r)), i))
            .collect(),
    )
}
pub(crate) fn pairs(a: &Index, b: &Index, tolerance: f64, same: bool) -> Vec<(usize, usize)> {
    use rstar::RTreeObject;
    let mut pairs = Vec::new();
    for item in a {
        let bounds = item.envelope();
        let rect = Rect::new(
            Coord {
                x: bounds.lower()[0],
                y: bounds.lower()[1],
            },
            Coord {
                x: bounds.upper()[0],
                y: bounds.upper()[1],
            },
        );
        for other in b.locate_in_envelope_intersecting(&expanded(rect, tolerance)) {
            if !same || item.data < other.data {
                pairs.push((item.data, other.data));
            }
        }
    }
    pairs.sort_unstable();
    pairs
}
