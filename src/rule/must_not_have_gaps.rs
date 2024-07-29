use crate::util::explode_linestrings;
use crate::{TopologyError, TopologyResult};
use geo::{GeoFloat, Contains, LineString, Polygon};
use rstar::{RTree, RTreeObject};

pub trait MustNotHaveGaps<T: GeoFloat> {
    fn must_not_have_gaps(self) -> TopologyResult<T>;
}

impl<T: GeoFloat + Send + Sync> MustNotHaveGaps<T> for Vec<Polygon<T>> {
    fn must_not_have_gaps(self) -> TopologyResult<T> {
        // Is to_owned ok here or not
        let boundaries = self
            .into_iter()
            .flat_map(|polygon| {
                polygon
                    .interiors()
                    .to_owned()
                    .into_iter()
                    .chain(std::iter::once(polygon.exterior().to_owned()))
            })
            .collect();
        let lines = explode_linestrings(&boundaries);
        let tree = RTree::bulk_load(lines);
        let results: Vec<LineString<T>> = tree
            .iter()
            .filter_map(|line| {
                let mut counter = 0;
                for other in tree.locate_in_envelope_intersecting(&line.envelope()) {
                    if line.contains(other) {
                        counter += 1
                    }
                    if counter == 2 {
                        return None;
                    }
                }
                Some(line.into())
            })
            .collect();
        if results.is_empty() {
            TopologyResult::Valid
        } else {
            TopologyResult::Errors(vec![TopologyError::LineString(results)])
        }
    }
}
