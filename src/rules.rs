mod there_are_no_dangles;
mod there_are_no_dangles_improved;
mod must_not_intersect;

pub use there_are_no_dangles::there_are_no_dangles;
pub use must_not_intersect::must_not_intersect;
pub use there_are_no_dangles_improved::there_are_no_dangles_improved;


#[derive(Debug)]
pub enum Rules {
    ThereAreNoDangles,
    MustNotIntersect
}

impl Rules {
    pub fn available(geometry: &geo_types::Geometry) -> Option<Vec<Rules>> {
        match geometry {
            geo_types::Geometry::LineString(_) | geo_types::Geometry::MultiLineString(_) => {
                Some(vec![Rules::ThereAreNoDangles, Rules::MustNotIntersect])
            },
            _ => None
        }
    }
}
