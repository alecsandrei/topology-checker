use crate::{GeometryError, TopologyResult};
use geo::{GeoFloat, Geometry};

pub trait MustNotBeMultipart<T: GeoFloat> {
    fn must_not_be_multipart(self) -> TopologyResult<T>;
}

impl<T: GeoFloat> MustNotBeMultipart<T> for Vec<Geometry<T>> {
    fn must_not_be_multipart(self) -> TopologyResult<T> {
        let mut multipoints = Vec::new();
        let mut multilinestrings = Vec::new();
        let mut multipolygons = Vec::new();
        self.into_iter().for_each(|geometry| match geometry {
            Geometry::MultiPoint(multipoint) => {
                multipoints.push(multipoint);
            }
            Geometry::MultiLineString(multilinestring) => {
                multilinestrings.push(multilinestring);
            }
            Geometry::MultiPolygon(multipolygon) => {
                multipolygons.push(multipolygon);
            }
            _ => (),
        });
        let mut geometry_errors = Vec::new();
        if !multipoints.is_empty() {
            geometry_errors.push(GeometryError::MultiPoint(multipoints));
        }
        if !multilinestrings.is_empty() {
            geometry_errors.push(GeometryError::MultiLineString(multilinestrings));
        }
        if !multipolygons.is_empty() {
            geometry_errors.push(GeometryError::MultiPolygon(multipolygons));
        }

        if geometry_errors.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(geometry_errors)
        }
    }
}
