use crate::util::{create_dataset, open_dataset, GdalDrivers};
use anyhow::Context;
use gdal::{
    errors::GdalError,
    spatial_ref::SpatialRef,
    vector::{LayerAccess, ToGdal},
    Dataset, LayerOptions, Metadata,
};
use geo::{
    GeoFloat, Geometry, Line, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon,
};
use geozero::{gdal::process_geom, geo_types::GeoWriter};
use std::{fmt::Display, path::PathBuf};

pub mod algorithm;
pub mod prelude;
pub mod rule;
pub mod util;

pub struct VectorDataset(Dataset);

impl VectorDataset {
    pub fn new(path: &PathBuf) -> anyhow::Result<Self> {
        Ok(VectorDataset(open_dataset(path)?))
    }

    pub fn to_geo(&self) -> anyhow::Result<Vec<Geometry<f64>>> {
        let mut layer = self
            .0
            .layers()
            .next()
            .expect(format!("Dataset {} has no layers.", self.0.description()?).as_str());
        let mut writer = GeoWriter::new();
        for feature in layer.features() {
            let geom = feature.geometry().unwrap();
            process_geom(geom, &mut writer).with_context(|| {
                format!(
                    "{} {}",
                    "Failed to parse FID",
                    feature
                        .fid()
                        .expect(format!("Failed to get FID of feature {:?}", feature).as_str()),
                )
            })?;
        }
        let geometry = writer.take_geometry();

        // If layer has more than 1 feature, it will match GeometryCollection.
        // Otherwise, it might match any of the rest.
        if let Some(geometry) = geometry {
            match geometry {
                geo::Geometry::GeometryCollection(geometry) => Ok(geometry.0),
                geo::Geometry::MultiLineString(geometry) => Ok(vec![geometry.into()]),
                geo::Geometry::MultiPolygon(geometry) => Ok(vec![geometry.into()]),
                geo::Geometry::MultiPoint(geometry) => Ok(vec![geometry.into()]),
                geo::Geometry::Point(geometry) => Ok(vec![geometry.into()]),
                geo::Geometry::LineString(geometry) => Ok(vec![geometry.into()]),
                geo::Geometry::Polygon(geometry) => Ok(vec![geometry.into()]),
                geo::Geometry::Line(geometry) => Ok(vec![geometry.into()]),
                _ => Err(anyhow::anyhow!(
                    "Did not expect the received geometry {:?}",
                    geometry
                )),
            }
        } else {
            Err(anyhow::anyhow!(
                "Failed to retrieve geometry. Is the dataset {} empty?",
                self.0.description()?
            ))
        }
    }

    pub fn srs(&self) -> anyhow::Result<Option<SpatialRef>> {
        let layer = self
            .0
            .layers()
            .next()
            .expect(format!("Dataset {} has no layers.", self.0.description()?).as_str());
        Ok(layer.spatial_ref())
    }

    pub fn compare_srs(&self, other: &VectorDataset) -> anyhow::Result<()> {
        if self.srs()? != other.srs()? {
            panic!(
                "{} does not have the same spatial reference system as {}",
                self.0.description().unwrap(),
                other.0.description().unwrap()
            )
        }
        Ok(())
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
pub enum TopologyError<T: GeoFloat> {
    Point(Vec<Point<T>>),
    LineString(Vec<LineString<T>>),
    Polygon(Vec<Polygon<T>>),
    MultiPoint(Vec<MultiPoint<T>>),
    MultiLineString(Vec<MultiLineString<T>>),
    MultiPolygon(Vec<MultiPolygon<T>>),
}

impl<T: GeoFloat> TopologyError<T> {
    fn len(&self) -> usize {
        match self {
            TopologyError::LineString(vec) => vec.len(),
            TopologyError::MultiLineString(vec) => vec.len(),
            TopologyError::MultiPoint(vec) => vec.len(),
            TopologyError::MultiPolygon(vec) => vec.len(),
            TopologyError::Point(vec) => vec.len(),
            TopologyError::Polygon(vec) => vec.len(),
        }
    }
}

impl<T: GeoFloat> Display for TopologyError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TopologyError::LineString(_) => write!(f, "{} LineString errors", self.len()),
            TopologyError::MultiLineString(_) => write!(f, "{} MultiLineString errors", self.len()),
            TopologyError::MultiPoint(_) => write!(f, "{} MultiPoint errors", self.len()),
            TopologyError::MultiPolygon(_) => write!(f, "{} MultiPolygon errors", self.len()),
            TopologyError::Point(_) => write!(f, "{} Point errors", self.len()),
            TopologyError::Polygon(_) => write!(f, "{} Polygon errors", self.len()),
        }
    }
}

impl<T: GeoFloat> TopologyError<T> {
    fn to_gdal(&self) -> anyhow::Result<Vec<gdal::vector::Geometry>, GdalError> {
        let geometries: anyhow::Result<Vec<_>, GdalError> = match self {
            Self::Point(points) => points.into_iter().map(|point| point.to_gdal()).collect(),
            Self::LineString(linestrings) => linestrings
                .into_iter()
                .map(|linestring| linestring.to_gdal())
                .collect(),
            Self::Polygon(polygons) => polygons
                .into_iter()
                .map(|polygon| polygon.to_gdal())
                .collect(),
            Self::MultiPoint(multipoints) => multipoints
                .into_iter()
                .map(|multipoint| multipoint.to_gdal())
                .collect(),
            Self::MultiLineString(multilinestrings) => multilinestrings
                .into_iter()
                .map(|multilinestring| multilinestring.to_gdal())
                .collect(),
            Self::MultiPolygon(multipolygons) => multipolygons
                .into_iter()
                .map(|multipolygon| multipolygon.to_gdal())
                .collect(),
        };
        Ok(geometries?)
    }
    pub fn export(&self, config: ExportConfig) -> anyhow::Result<()> {
        let ExportConfig {
            rule_name,
            output,
            mut options,
            mut dataset,
        } = config;
        // We make this created_dataset object to store the
        // created dataset. This makes the possibly created dataset
        // live long enough.
        let mut created_dataset = None;
        {
            // This scope creates a dataset in case it was not provided.
            if dataset.is_none() && output.is_some() {
                let _ = created_dataset.insert(
                    create_dataset(output.unwrap(), None)
                        .with_context(|| format!("Failed to create the dataset at {output:?}."))?,
                );
                let _ = dataset.insert(created_dataset.as_mut().unwrap());
            }
        }
        if dataset.as_ref().is_some() {
            let geometries: Vec<gdal::vector::Geometry> = self.to_gdal()?;
            let geometry_type = geometries[0].geometry_type();
            let geometry_name = geometries[0].geometry_name();
            let mut layer = None;
            if let Some(ref dataset) = &mut dataset {
                layer = dataset.layers().find_map(|layer| {
                    let field_type = layer.defn().geom_fields().next().unwrap().field_type();
                    if field_type == geometry_type {
                        return Some(layer);
                    }
                    None
                });
            }
            if let Some(mut layer) = layer {
                geometries
                    .into_iter()
                    .map(|geom| -> anyhow::Result<()> {
                        Ok(layer.create_feature_fields(
                            geom,
                            &["rule"],
                            &[gdal::vector::FieldValue::StringValue(rule_name.to_string())],
                        )?)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
            } else {
                let mut layer = None;
                if let Some(dataset) = &mut dataset {
                    options.name = &geometry_name;
                    options.ty = geometry_type;
                    let _ = layer.insert(
                        dataset
                            .create_layer(options.clone())
                            .with_context(|| "Failed to create a layer inside the dataset.")?,
                    );
                }
                let field =
                    gdal::vector::FieldDefn::new("rule", gdal::vector::OGRFieldType::OFTString)
                        .with_context(|| "Failed to create field 'rule' inside the layer.")?;
                field.add_to_layer(layer.as_mut().unwrap()).unwrap();
                geometries
                    .into_iter()
                    .map(|geom| -> anyhow::Result<()> {
                        Ok(layer.as_mut().unwrap().create_feature_fields(
                            geom,
                            &["rule"],
                            &[gdal::vector::FieldValue::StringValue(rule_name.to_string())],
                        )?)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
            }
        }
        Ok(())
    }
}

pub enum TopologyResult<T: GeoFloat> {
    Errors(Vec<TopologyError<T>>),
    Valid,
}

pub struct ExportConfig<'a> {
    pub rule_name: String,
    pub output: Option<&'a PathBuf>,
    pub options: LayerOptions<'a>,
    pub dataset: Option<&'a mut Dataset>,
}

impl<'a> Default for ExportConfig<'a> {
    fn default() -> Self {
        ExportConfig {
            rule_name: String::new(),
            output: None,
            options: LayerOptions {
                ..Default::default()
            },
            dataset: None,
        }
    }
}

// Cloning only works if the field dataset is None.
impl<'a> Clone for ExportConfig<'a> {
    fn clone(&self) -> Self {
        if self.dataset.is_some() {
            panic!("Can not clone ExportConfig when the field Dataset is Some()")
        }
        Self {
            rule_name: self.rule_name.clone(),
            output: self.output.clone(),
            options: self.options.clone(),
            ..Default::default()
        }
    }
}

impl<T: GeoFloat> TopologyResult<T> {
    pub fn unwrap_err(&self) -> &Vec<TopologyError<T>> {
        match self {
            Self::Errors(geometry_errors) => geometry_errors,
            Self::Valid => panic!("Called unwrap_err on a Valid variant."),
        }
    }

    pub fn summary(&self, rule_name: Option<String>) {
        if let Some(rule_name) = rule_name {
            println!("{:-^60}", rule_name);
        } else {
            println!("{:-^60}", "");
        }
        let bar = "|";
        if self.is_valid() {
            println!("{: <30}{: >30}", "| No topology errors found.", bar);
        } else {
            for error in self.unwrap_err() {
                println!("{: <30}{: >30}", format!("| {}", error), bar);
            }
        }
        println!("{:-^60}", "");
    }

    pub fn unwrap_err_point(&self) -> &TopologyError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let TopologyError::Point(_) = error {
                    return true;
                }
                false
            })
            .expect("No point errors exist.")
    }

    pub fn unwrap_err_linestring(&self) -> &TopologyError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let TopologyError::LineString(_) = error {
                    return true;
                }
                false
            })
            .expect("No linestring errors exist.")
    }

    pub fn unwrap_err_polygon(&self) -> &TopologyError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let TopologyError::Polygon(_) = error {
                    return true;
                }
                false
            })
            .expect("No polygon errors exist.")
    }

    pub fn unwrap_err_multipoint(&self) -> &TopologyError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let TopologyError::MultiPoint(_) = error {
                    return true;
                }
                false
            })
            .expect("No multipoint errors exist.")
    }

    pub fn unwrap_err_multilinestring(&self) -> &TopologyError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let TopologyError::MultiLineString(_) = error {
                    return true;
                }
                false
            })
            .expect("No multilinestring errors exist.")
    }

    pub fn unwrap_err_multipolygon(&self) -> &TopologyError<T> {
        self.unwrap_err()
            .into_iter()
            .find(|error| {
                if let TopologyError::MultiPolygon(_) = error {
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
}

type RuleName = String;

pub struct TopologyResults<T: GeoFloat>(pub Vec<(RuleName, TopologyResult<T>)>);

impl<T: GeoFloat> TopologyResults<T> {
    pub fn new(results: Vec<(RuleName, TopologyResult<T>)>) -> Self {
        TopologyResults(results)
    }
}

impl<T: GeoFloat> TopologyResults<T> {
    pub fn export(self, output: &PathBuf) -> anyhow::Result<()> {
        let driver = gdal::DriverManager::get_driver_by_name(
            &GdalDrivers.infer_driver_name("gpkg").unwrap().0,
        )
        .expect("Error with getting the driver name from 'gpkg' abbreviation");
        let mut dataset = driver
            .create_vector_only(output)
            .with_context(|| format!("Failed to create geopackage at {output:?}."))?;
        let mut txn = dataset
            .start_transaction()
            .with_context(|| "Failed to start transaction (writing into the geopackage).")?;
        for result in self.0 {
            result.1.summary(Some(result.0.clone()));
            if !result.1.is_valid() {
                // Iterate and export all of the errors
                for error in result.1.unwrap_err() {
                    let config = ExportConfig {
                        rule_name: result.0.clone(),
                        dataset: Some(&mut txn),
                        ..Default::default()
                    };
                    error.export(config)?
                }
            }
        }
        txn.commit().expect("Failed to commit changes.");
        Ok(())
    }
}

#[cfg(test)]
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
