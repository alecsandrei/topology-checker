use geo::GeoFloat;
use crate::TopologyResult;

pub trait MustBeOutside<T: GeoFloat> {
    fn must_not_be_multipart(self) -> TopologyResult<T>;
}