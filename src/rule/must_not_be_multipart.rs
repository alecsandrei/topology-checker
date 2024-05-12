use geo::{GeoFloat, Geometry};

pub trait MustNotBeMultipart<T: GeoFloat> {
    fn must_not_be_multipart(self) -> Vec<Geometry<T>>;
}

impl<T: GeoFloat> MustNotBeMultipart<T> for Vec<Geometry<T>> {
    fn must_not_be_multipart(self) -> Vec<Geometry<T>> {
        self.into_iter().filter(|geometry| {
            match geometry {
                Geometry::MultiLineString(_) => true,
                Geometry::MultiPoint(_) => true,
                Geometry::MultiPolygon(_) => true,
                _ => false
            }
        }).collect()
    }
}