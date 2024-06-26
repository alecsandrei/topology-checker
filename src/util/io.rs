use gdal::{
    errors::GdalError, vector::LayerAccess, Dataset, DatasetOptions, GdalOpenFlags, LayerOptions,
    Metadata,
};
use std::collections::HashMap;
use std::path::PathBuf;
use colored::Colorize;

pub fn open_dataset(path: &PathBuf) -> anyhow::Result<Dataset> {
    if !path.exists() {
        return Err(anyhow::anyhow!("The provided path {:?} does not exist", path));
    }
    let options = DatasetOptions {
        open_flags: GdalOpenFlags::GDAL_OF_VECTOR,
        ..Default::default()
    };

    let dataset = Dataset::open_ex(path, options)?;

    Ok(dataset)
}

pub fn create_dataset(out_path: &PathBuf, driver: Option<String>) -> Result<Dataset, GdalError> {
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

    drv.create_vector_only(out_path)
}

pub fn geometries_to_file(
    geometries: Vec<gdal::vector::Geometry>,
    out_path: &PathBuf,
    driver: Option<String>,
    options: Option<LayerOptions>,
) {
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

pub struct GdalDrivers;
type DriverProps = HashMap<&'static str, Option<String>>;

impl GdalDrivers {
    pub fn infer_driver_name(&self, extension: &str) -> Option<(String, DriverProps)> {
        // Finds out whether or not the input file suffix can be mapped to a valid driver.
        self.driver_map().into_iter().find(|(_, properties)| {
            if properties
                .get("extensions")
                .unwrap()
                .clone()
                .unwrap()
                .contains(extension)
            {
                return true;
            }
            false
        })
    }

    fn driver_map(&self) -> HashMap<String, DriverProps> {
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
                if !extension.is_empty()
                    && driver.metadata_item("DCAP_VECTOR", "").is_some()
                    && !driver.short_name().is_empty()
                {
                    drivers.insert(driver.short_name(), properties);
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
