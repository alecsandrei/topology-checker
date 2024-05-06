use gdal::vector::ToGdal;
use gdal::{vector::LayerAccess, Dataset, LayerOptions, Metadata};
use std::collections::HashMap;

pub fn open_dataset(path: &str) -> Dataset {
    Dataset::open(path).expect(format!("Could not read file {path}.").as_str())
}

pub fn geometries_to_file<G>(
    geometries: Vec<G>,
    out_path: &str,
    driver: Option<String>,
    options: Option<LayerOptions>,
) where
    G: ToGdal,
{
    let geometries: Vec<gdal::vector::Geometry> = geometries
        .into_iter()
        .map(|geometry| geometry.to_gdal().unwrap())
        .collect();
    let driver_name = driver.unwrap_or_else(|| {
        let mut driver_map = GdalDrivers.write();
        driver_map.remove(out_path.split('.').last().unwrap()).expect(
            "Could not infer driver by file extension. Consider specifying the GDAL_DRIVER environment variable."
    )});
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

pub struct GdalDrivers;

impl GdalDrivers {
    fn driver_map(&self) -> HashMap<String, HashMap<&'static str, Option<String>>> {
        let mut drivers = HashMap::new();
        for i in 0..gdal::DriverManager::count() {
            let driver = gdal::DriverManager::get_driver(i).unwrap();
            let mut extension = driver.metadata_item("DMD_EXTENSION", "");
            if let Some(extensions) = driver.metadata_item("DMD_EXTENSIONS", "") {
                // DMD_EXTENSIONS takes priority over DMD_EXTENSION
                if !extensions.is_empty() {
                    extension = Some(extensions)
                }
            }
            let mut properties = HashMap::new();
            properties.insert("read", driver.metadata_item("DCAP_OPEN", ""));
            properties.insert("write", driver.metadata_item("DCAP_CREATE", ""));
            properties.insert("extensions", extension);

            if let Some(extension) = properties.get("extensions").unwrap() {
                if !extension.is_empty() && driver.metadata_item("DCAP_VECTOR", "").is_some() {
                    let mut name = driver.long_name();
                    if name.is_empty() {
                        name = driver.short_name()
                    }
                    drivers.insert(name, properties);
                }
            }
        }
        drivers
    }

    pub fn read_write(&self) -> HashMap<String, String> {
        self.driver_map()
            .into_iter()
            .filter_map(|(driver, properties)| {
                if properties.get("read").unwrap().is_some()
                    && properties.get("write").unwrap().is_some()
                {
                    return Some((
                        driver,
                        properties.get("extensions").unwrap().clone().unwrap(),
                    ));
                }
                None
            })
            .collect()
    }

    pub fn read(&self) -> HashMap<String, String> {
        self.driver_map()
            .into_iter()
            .filter_map(|(driver, properties)| {
                if properties.get("read").unwrap().is_some() {
                    return Some((
                        driver,
                        properties.get("extensions").unwrap().clone().unwrap(),
                    ));
                }
                None
            })
            .collect()
    }

    pub fn write(&self) -> HashMap<String, String> {
        self.driver_map()
            .into_iter()
            .filter_map(|(driver, properties)| {
                if properties.get("write").unwrap().is_some() {
                    return Some((
                        driver,
                        properties.get("extensions").unwrap().clone().unwrap(),
                    ));
                }
                None
            })
            .collect()
    }
}
