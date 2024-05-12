use crate::util::open_dataset;
use gdal::{spatial_ref::SpatialRef, vector::LayerAccess, Dataset, Metadata};
use geo::{GeoFloat, Geometry, Line, LineString, MultiPolygon, Polygon};
use geozero::{gdal::process_geom, geo_types::GeoWriter};

pub mod algorithm;
pub mod prelude;
pub mod rule;
pub mod util;

pub struct VectorDataset(Dataset);

impl VectorDataset {
    pub fn new(path: &str) -> Self {
        VectorDataset(open_dataset(path))
    }

    pub fn to_geo(&self) -> geozero::error::Result<Vec<Geometry<f64>>> {
        let mut layer =
            self.0.layers().next().expect(
                format!("Dataset {} has no layers.", self.0.description().unwrap()).as_str(),
            );
        let mut writer = GeoWriter::new();
        for feature in layer.features() {
            let geom = feature.geometry().unwrap();
            process_geom(geom, &mut writer)?;
        }
        let geometry = writer.take_geometry().unwrap();
        match geometry {
            geo::Geometry::GeometryCollection(geometry) => Ok(geometry.0),
            _ => unreachable!(),
        }
    }

    pub fn crs(&self) -> Option<SpatialRef> {
        let layer =
            self.0.layers().next().expect(
                format!("Dataset {} has no layers.", self.0.description().unwrap()).as_str(),
            );
        layer.spatial_ref()
    }
}

pub trait GeometryType<T:GeoFloat> {}

impl<T:GeoFloat> GeometryType<T> for Geometry<T> {}
impl<T:GeoFloat> GeometryType<T> for MultiPolygon<T> {}
impl<T:GeoFloat> GeometryType<T> for Polygon<T> {}
impl<T:GeoFloat> GeometryType<T> for LineString<T> {}
impl<T:GeoFloat> GeometryType<T> for Line<T> {}

mod tests {
    // Test for the README.md file.
    #[cfg(doctest)]
    mod test_readme {
        macro_rules! external_doc_test {
            ($x:expr) => {
                #[doc = $x]
                extern "C" {}
            };
        }

        external_doc_test!(include_str!("../README.md"));
    }
}
