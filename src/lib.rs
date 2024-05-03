use core::panic;
use gdal::{
    vector::{LayerAccess, ToGdal},
    Dataset, LayerOptions,
};
use geozero::{gdal::process_geom, geo_types::GeoWriter};

pub mod rules;
pub mod utils;

pub struct VectorDataset(Dataset);

impl VectorDataset {
    pub fn new(path: &str) -> Self {
        VectorDataset(open_dataset(path))
    }

    pub fn from_gdal(&self) -> Vec<geo::Geometry> {
        let mut layer = self.0.layers().next().unwrap();
        let mut writer = GeoWriter::new();
        for feature in layer.features() {
            let geom = feature.geometry().unwrap();
            process_geom(geom, &mut writer).unwrap();
        }
        let geometry = writer.take_geometry().unwrap();
        match geometry {
            geo::Geometry::GeometryCollection(geometry) => geometry.0,
            _ => panic!("Wrong geometry input."),
        }
    }
}

pub fn geometries_to_file<G>(geometries: Vec<G>, out_path: &str)
where
    G: ToGdal
{
    let geometries: Vec<gdal::vector::Geometry> = geometries
        .into_iter()
        .map(|geometry| geometry.to_gdal().unwrap())
        .collect();
    let drv = gdal::DriverManager::get_driver_by_name("ESRI Shapefile").unwrap();
    let mut ds = drv.create_vector_only(out_path).unwrap();
    let mut lyr = ds
        .create_layer(LayerOptions {
            name: "dangles",
            srs: geometries.first().unwrap().spatial_ref().as_ref(),
            ..Default::default()
        })
        .unwrap();
    geometries.into_iter().for_each(|geom| {
        lyr.create_feature(geom).expect("Couldn't write geometry");
    });
}

fn open_dataset(path: &str) -> Dataset {
    Dataset::open(path).expect("Could not read file.")
}
