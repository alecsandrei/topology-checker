use crate::model::{ValidationResult, geometry_json};
use anyhow::{Context, Result};
use gdal::{
    DriverManager,
    spatial_ref::CoordTransform,
    vector::{Feature, LayerAccess, LayerOptions, OGRFieldType, OGRwkbGeometryType, ToGdal},
};
use serde_json::{Value, json};
use std::path::Path;
use topology_checker_io::crs;

/// Stream a complete report without materializing all serialized violations.
///
/// # Errors
/// Returns an error for output failures or violation serialization failures.
pub fn write_json(result: &ValidationResult, mut output: impl std::io::Write) -> Result<()> {
    let summary = result.summary();
    output.write_all(b"{")?;
    for (i, (key, value)) in summary.as_object().unwrap().iter().enumerate() {
        if i > 0 {
            output.write_all(b",")?;
        }
        serde_json::to_writer(&mut output, key)?;
        output.write_all(b":")?;
        serde_json::to_writer(&mut output, value)?;
    }
    output.write_all(b",\"violations\":[")?;
    for i in 0..result.report.error_count() {
        if i > 0 {
            output.write_all(b",")?;
        }
        serde_json::to_writer(&mut output, &result.violation(i, false)?)?;
    }
    output.write_all(b"]}")?;
    output.flush()?;
    Ok(())
}

/// Stream WGS84 GeoJSON with one feature per violation.
///
/// # Errors
/// Returns an error for missing CRS, transformation failures or output failures.
pub fn write_geojson(result: &ValidationResult, mut output: impl std::io::Write) -> Result<()> {
    let from = crs::parse(
        result
            .config
            .working_crs
            .as_deref()
            .context("assign a working CRS before geographic export")?,
    )?;
    let operation = CoordTransform::new(&from, &crs::parse("EPSG:4326")?)?;
    output.write_all(b"{\"type\":\"FeatureCollection\",\"features\":[")?;
    for (i, e) in result.report.errors().enumerate() {
        if i > 0 {
            output.write_all(b",")?;
        }
        let mut properties = result.violation(i, false)?;
        properties.as_object_mut().unwrap().remove("geometry");
        let feature = json!({"type":"Feature","id":i.to_string(),"geometry":geometry_json(&crs::transform(&e.geometry,&operation)?),"properties":properties});
        serde_json::to_writer(&mut output, &feature)?;
    }
    output.write_all(b"],\"summary\":")?;
    serde_json::to_writer(&mut output, &result.summary())?;
    output.write_all(b"}")?;
    output.flush()?;
    Ok(())
}

/// Export one WGS84 GeoJSON feature per aggregated violation.
///
/// # Errors
/// Returns an error for unknown working CRS or failed coordinate transformations.
pub fn geojson(result: &ValidationResult) -> Result<Value> {
    let from = crs::parse(
        result
            .config
            .working_crs
            .as_deref()
            .context("assign a working CRS before geographic export")?,
    )?;
    let operation = CoordTransform::new(&from, &crs::parse("EPSG:4326")?)?;
    let features = result.report.errors().enumerate().map(|(i,e)| {
        let mut properties = result.violation(i,false)?;
        properties.as_object_mut().unwrap().remove("geometry");
        Ok(json!({"type":"Feature","id":i.to_string(),"geometry":geometry_json(&crs::transform(&e.geometry,&operation)?),"properties":properties}))
    }).collect::<Result<Vec<_>>>()?;
    Ok(json!({"type":"FeatureCollection","features":features,"summary":result.summary()}))
}
/// One record per violation, including multipart and mixed collections.
/// Export mixed violations to a single GeoPackage layer.
///
/// # Errors
/// Returns an error for unknown working CRS or GDAL write failures.
pub fn geopackage(result: &ValidationResult, path: &Path) -> Result<()> {
    let srs = crs::parse(
        result
            .config
            .working_crs
            .as_deref()
            .context("assign a working CRS before geographic export")?,
    )?;
    let mut dataset = DriverManager::get_driver_by_name("GPKG")?.create_vector_only(path)?;
    let layer = dataset.create_layer(LayerOptions {
        name: "violations",
        srs: Some(&srs),
        ty: OGRwkbGeometryType::wkbUnknown,
        ..Default::default()
    })?;
    layer.create_defn_fields(&[
        ("violation_id", OGRFieldType::OFTInteger64),
        ("rule", OGRFieldType::OFTString),
        ("category", OGRFieldType::OFTString),
        ("source_refs", OGRFieldType::OFTString),
    ])?;
    for (i, e) in result.report.errors().enumerate() {
        let row = result.violation(i, false)?;
        let mut feature = Feature::new(layer.defn())?;
        feature.set_geometry(e.geometry.to_gdal()?)?;
        feature.set_field_integer64(0, i as i64)?;
        feature.set_field_string(1, e.rule.name())?;
        feature.set_field_string(2, e.error_type.name())?;
        feature.set_field_string(
            3,
            &json!({"origin":row["origin"],"destination":row["destination"]}).to_string(),
        )?;
        feature.create(&layer)?;
    }
    dataset.flush_cache()?;
    Ok(())
}
