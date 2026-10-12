use super::precision::Geometry;
use crate::model::Parts;

#[derive(Debug)]
pub(crate) struct Union {
    pub geometry: Geometry,
}
pub(crate) fn union(parts: &Parts) -> &Union {
    parts.union.get_or_init(|| Union {
        geometry: super::arrangement::overlay(
            parts.polygons.iter().map(|p| p.geometry.as_ref()),
            std::iter::empty(),
            false,
        ),
    })
}
