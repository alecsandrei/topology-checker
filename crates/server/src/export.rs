use crate::{
    dto::{GeoJson, SourceRef, Summary, Violation},
    model::{ValidationResult, geometry},
};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use topology_checker_core::{Rule, ViolationKind};
use topology_checker_io::crs;
use topology_checker_io::{Transform as CoordTransform, write};

/// A violation without geometry, as GeoJSON feature properties.
#[derive(Debug, Serialize)]
struct Properties {
    id: usize,
    rule: Rule,
    category: ViolationKind,
    origin: Option<SourceRef>,
    destination: Option<SourceRef>,
}
impl From<Violation> for Properties {
    fn from(v: Violation) -> Self {
        Self {
            id: v.id,
            rule: v.rule,
            category: v.category,
            origin: v.origin,
            destination: v.destination,
        }
    }
}
#[derive(Debug, Serialize)]
struct Feature {
    r#type: &'static str,
    id: String,
    geometry: GeoJson,
    properties: Properties,
}
#[derive(Debug, Serialize)]
struct FeatureCollection {
    r#type: &'static str,
    features: Vec<Feature>,
    summary: Summary,
}

/// Stream a complete report without materializing all serialized violations.
///
/// # Errors
/// Returns an error for output failures or violation serialization failures.
pub fn write_json(result: &ValidationResult, mut output: impl std::io::Write) -> Result<()> {
    let Value::Object(summary) = serde_json::to_value(result.summary())? else {
        unreachable!("a summary serializes as an object");
    };
    output.write_all(b"{")?;
    for (i, (key, value)) in summary.iter().enumerate() {
        if i > 0 {
            output.write_all(b",")?;
        }
        serde_json::to_writer(&mut output, key)?;
        output.write_all(b":")?;
        serde_json::to_writer(&mut output, value)?;
    }
    output.write_all(b",\"violations\":[")?;
    for i in 0..result.report.violation_count() {
        if i > 0 {
            output.write_all(b",")?;
        }
        serde_json::to_writer(&mut output, &result.violation(i, false)?)?;
    }
    output.write_all(b"]}")?;
    output.flush()?;
    Ok(())
}

fn wgs84(result: &ValidationResult) -> Result<CoordTransform> {
    let from = crs::parse(
        result
            .config
            .working_crs
            .as_deref()
            .context("assign a working CRS before geographic export")?,
    )?;
    CoordTransform::new(&from, &crs::parse("EPSG:4326")?)
}
/// Build WGS84 features lazily, in report order.
fn features<'a>(
    result: &'a ValidationResult,
    operation: &'a CoordTransform,
) -> impl Iterator<Item = Result<Feature>> + 'a {
    result.report.violations().enumerate().map(move |(i, e)| {
        Ok(Feature {
            r#type: "Feature",
            id: i.to_string(),
            geometry: geometry(&crs::transform(&e.geometry, operation)?),
            properties: result.violation(i, false)?.into(),
        })
    })
}

/// Stream WGS84 GeoJSON with one feature per violation.
///
/// # Errors
/// Returns an error for missing CRS, transformation failures or output failures.
pub fn write_geojson(result: &ValidationResult, mut output: impl std::io::Write) -> Result<()> {
    let operation = wgs84(result)?;
    output.write_all(b"{\"type\":\"FeatureCollection\",\"features\":[")?;
    for (i, feature) in features(result, &operation).enumerate() {
        if i > 0 {
            output.write_all(b",")?;
        }
        serde_json::to_writer(&mut output, &feature?)?;
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
    let operation = wgs84(result)?;
    Ok(serde_json::to_value(FeatureCollection {
        r#type: "FeatureCollection",
        features: features(result, &operation).collect::<Result<_>>()?,
        summary: result.summary(),
    })?)
}
/// Export one record per violation, keeping multipart and mixed geometries whole,
/// with the report columns the CLI writes.
///
/// # Errors
/// Returns an error for a missing working CRS, geometry the format cannot hold or
/// dataset write failures.
pub fn dataset(result: &ValidationResult, path: &Path) -> Result<()> {
    let srs = crs::parse(
        result
            .config
            .working_crs
            .as_deref()
            .context("assign a working CRS before geographic export")?,
    )?;
    write::report(
        path,
        None,
        false,
        Some(&srs),
        &result.report,
        write::ReportLayout::Complete,
    )?;
    Ok(())
}
