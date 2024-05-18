use crate::util::{
    open_dataset, GdalDrivers,
};
use gdal::{
    spatial_ref::SpatialRef,
    vector::{LayerAccess, ToGdal},
    Dataset, LayerOptions, Metadata,
};
use geo::{
    GeoFloat, Geometry, Line, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon,
};
use geozero::{gdal::process_geom, geo_types::GeoWriter};
use std::path::PathBuf;

pub mod algorithm;
pub mod prelude;
pub mod rule;
pub mod util;

pub struct VectorDataset(Dataset);

impl VectorDataset {
    pub fn new(path: &PathBuf) -> Self {
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

pub trait GeometryType<T: GeoFloat> {}

impl<T: GeoFloat> GeometryType<T> for Geometry<T> {}
impl<T: GeoFloat> GeometryType<T> for Point<T> {}
impl<T: GeoFloat> GeometryType<T> for Line<T> {}
impl<T: GeoFloat> GeometryType<T> for LineString<T> {}
impl<T: GeoFloat> GeometryType<T> for MultiPolygon<T> {}
impl<T: GeoFloat> GeometryType<T> for Polygon<T> {}

#[derive(Hash, PartialEq, Eq, Debug)]
pub enum GeometryError<T: GeoFloat> {
    Point(Vec<Point<T>>),
    LineString(Vec<LineString<T>>),
    Polygon(Vec<Polygon<T>>),
    MultiPoint(Vec<MultiPoint<T>>),
    MultiLineString(Vec<MultiLineString<T>>),
    MultiPolygon(Vec<MultiPolygon<T>>),
}

impl<T: GeoFloat> GeometryError<T> {
    fn to_gdal(&self) -> Vec<gdal::vector::Geometry> {
        match self {
            Self::Point(points) => points
                .into_iter()
                .map(|point| point.to_gdal().expect("Failed to convert to GDAL."))
                .collect(),
            Self::LineString(linestrings) => linestrings
                .into_iter()
                .map(|linestring| linestring.to_gdal().expect("Failed to convert to GDAL."))
                .collect(),
            Self::Polygon(polygons) => polygons
                .into_iter()
                .map(|polygon| polygon.to_gdal().expect("Failed to convert to GDAL."))
                .collect(),
            Self::MultiPoint(multipoints) => multipoints
                .into_iter()
                .map(|multipoint| multipoint.to_gdal().expect("Failed to convert to GDAL."))
                .collect(),
            Self::MultiLineString(multilinestrings) => multilinestrings
                .into_iter()
                .map(|multilinestring| {
                    multilinestring
                        .to_gdal()
                        .expect("Failed to convert to GDAL.")
                })
                .collect(),
            Self::MultiPolygon(multipolygons) => multipolygons
                .into_iter()
                .map(|multipolygon| multipolygon.to_gdal().expect("Failed to convert to GDAL."))
                .collect(),
        }
    }
    pub fn to_file(
        self,
        out_path: &PathBuf,
        driver: Option<String>,
        options: Option<LayerOptions>,
    ) {
        let geometries = self.to_gdal();
        // If driver is not provided, attempt to infer it from the file extension.
        let driver_name = driver.unwrap_or_else(|| {
        let driver = GdalDrivers
            .infer_driver_name(out_path.extension().expect(format!("Path {out_path:?} does not have a valid extension.").as_str()).to_str().unwrap())
            .expect("Could not infer driver by file extension. Consider specifying the GDAL_DRIVER environment variable.");
        driver.1.get("write").unwrap().clone().expect(format!("Driver {} is not writeable.", driver.0).as_str());
        driver.0
    });
        let drv = gdal::DriverManager::get_driver_by_name(&driver_name)
            .expect(format!("Driver {driver_name} does not exist.").as_str());

        let mut ds = drv.create_vector_only(out_path).unwrap();
        let options = options.unwrap_or(LayerOptions {
            ..Default::default()
        });
        let mut lyr = ds.create_layer(options).unwrap();
        geometries.into_iter().for_each(|geom| {
            lyr.create_feature(geom).expect("Couldn't write geometry");
        });
    }
}

pub enum TopologyResult<T: GeoFloat> {
    Errors(Vec<GeometryError<T>>),
    Valid,
}

impl<T: GeoFloat> TopologyResult<T> {
    pub fn unwrap_err(self) -> Vec<GeometryError<T>> {
        match self {
            Self::Errors(geometry_errors) => geometry_errors,
            Self::Valid => panic!("Called unwrap_err on a Valid variant."),
        }
    }

    pub fn unwrap_err_point(self) -> GeometryError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let GeometryError::Point(_) = error {
                    return true;
                }
                false
            })
            .expect("No point errors exist.")
    }

    pub fn unwrap_err_linestring(self) -> GeometryError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let GeometryError::LineString(_) = error {
                    return true;
                }
                false
            })
            .expect("No linestring errors exist.")
    }

    pub fn unwrap_err_polygon(self) -> GeometryError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let GeometryError::Polygon(_) = error {
                    return true;
                }
                false
            })
            .expect("No polygon errors exist.")
    }

    pub fn unwrap_err_multipoint(self) -> GeometryError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let GeometryError::MultiPoint(_) = error {
                    return true;
                }
                false
            })
            .expect("No multipoint errors exist.")
    }

    pub fn unwrap_err_multilinestring(self) -> GeometryError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let GeometryError::MultiLineString(_) = error {
                    return true;
                }
                false
            })
            .expect("No multilinestring errors exist.")
    }

    pub fn unwrap_err_multipolygon(self) -> GeometryError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let GeometryError::MultiPolygon(_) = error {
                    return true;
                }
                false
            })
            .expect("No multipolygon errors exist.")
    }

    pub fn is_valid(&self) -> bool {
        match self {
            Self::Valid => true,
            Self::Errors(_) => false,
        }
    }

    // pub fn write_report(
    //     self,
    //     out_dir: &PathBuf,
    //     driver: Option<String>,
    //     options: Option<LayerOptions>,
    // ) {

    //     let dataset = RefCell::new(create_dataset(
    //         out_dir.join("/errors"),
    //         driver,
    //     ));

    //     let mut points_dataset = dataset.borrow_mut();
    //     let mut lines_dataset = dataset.borrow_mut();
    //     let mut polygons_dataset = dataset.borrow_mut();

    //     let mut points_layer = None;
    //     let mut lines_layer = None;
    //     let mut polygons_layer = None;

    //     fn write_to_layer<T: GeoFloat>(layer: &mut Layer<'_>, geom: Geometry<T>) {
    //         layer
    //             .create_feature(
    //                 geom.to_gdal()
    //                     .expect("Failed to convert geo-types to GDAL."),
    //             )
    //             .expect("Failed to write point geometry.")
    //     }

    //     for geom in self.unwrap_err() {
    //         if is_point(&geom) {
    //             if points_layer.is_none() {
    //                 points_layer = Some(create_layer(&mut points_dataset, options.clone()));
    //             }
    //             write_to_layer(&mut points_layer.unwrap(), geom)
    //         } else if is_line(&geom) {
    //             if polygons_layer.is_none() {
    //                 polygons_layer = Some(create_layer(&mut lines_dataset, options.clone()));
    //             }
    //         } else if is_polygon(&geom) {
    //             if lines_layer.is_none() {
    //                 lines_layer = Some(create_layer(&mut polygons_dataset, options.clone()));
    //             }
    //         }
    //     }
    // }
}

#[cfg(test)]
mod tests {

    // use super::*;
    // use geo::{line_string, point, polygon};

    // fn topology_result() {
    //     let points =
    //         GeometryCollection([point! { x: 181.2, y: 51.79 }, point! { x: 181.2, y: 51.79 }]);
    //     let line_strings = vec![
    //         line_string![
    //             (x: -21.95156, y: 64.1446),
    //             (x: -21.951, y: 64.14479)],
    //         line_string![
    //             (x: -21.95156, y: 64.1446),
    //             (x: -21.951, y: 64.14479)],
    //     ];
    //     let polygons = vec![polygon![
    //         (x: -111., y: 45.),
    //         (x: -111., y: 41.),
    //         (x: -104., y: 41.),
    //         (x: -104., y: 45.)
    //     ]];
    //     let errors: GeometryCollection<f64> =
    //         points.into_iter().chain(line_strings.into_iter()).collect();
    // }

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
