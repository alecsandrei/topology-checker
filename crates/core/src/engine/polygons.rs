use crate::{
    engine::index::{self, Index},
    model::Parts,
};
use geo::{
    Area, BoundingRect, Euclidean, Length, MultiPolygon, Polygon,
    orient::{Direction, Orient},
    unary_union,
};

#[derive(Debug)]
pub(crate) struct Union {
    pub geometry: MultiPolygon,
    pub index: Index,
}
pub(crate) fn union(parts: &Parts) -> &Union {
    parts.union.get_or_init(|| {
        let polygons = parts
            .polygons
            .iter()
            .map(|p| p.geometry.orient(Direction::Default))
            .collect::<Vec<_>>();
        let geometry = unary_union(&polygons);
        let index = index::build(
            geometry
                .0
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.bounding_rect().map(|r| (i, r))),
        );
        Union { geometry, index }
    })
}
pub(crate) fn substantial(polygon: &Polygon, tolerance: f64) -> bool {
    let area = polygon.unsigned_area();
    let perimeter = Euclidean.length(polygon.exterior())
        + polygon
            .interiors()
            .iter()
            .map(|r| Euclidean.length(r))
            .sum::<f64>();
    area > 0. && perimeter > 0. && 2. * area / perimeter >= tolerance
}
