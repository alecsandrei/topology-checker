mod there_are_no_dangles;
mod must_not_intersect;
mod must_not_overlap;

pub use there_are_no_dangles::there_are_no_dangles;
pub use must_not_intersect::must_not_intersect;
pub use must_not_overlap::must_not_overlap;

#[derive(Debug)]
pub enum Rules {
    ThereAreNoDangles,
    MustNotIntersect,
    MustNotOverlap
}

impl Rules {
    pub fn available(geometry: &geo_types::Geometry) -> Option<Vec<Rules>> {
        match geometry {
            geo_types::Geometry::LineString(_) | geo_types::Geometry::MultiLineString(_) => {
                Some(vec![Rules::ThereAreNoDangles, Rules::MustNotIntersect])
            },
            geo_types::Geometry::Polygon(_) | geo_types::Geometry::MultiPolygon(_) => {
                Some(vec![Rules::MustNotOverlap])
            },
            _ => None
        }
    }
}
