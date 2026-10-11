//! Shared capabilities for CLI, web imports and dataset exports.
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Format {
    pub name: String,
    pub extensions: Vec<String>,
    pub read: bool,
    pub write: bool,
    pub layers: bool,
    pub collections: bool,
    pub polygons: bool,
    pub web_export: bool,
    pub native: bool,
}
pub fn formats() -> Vec<Format> {
    let mut formats = [
        ("GeoJSON", &["geojson", "json"][..], false, true, true, true),
        ("ESRI Shapefile", &["shp"][..], false, false, true, false),
        ("FlatGeobuf", &["fgb"][..], false, true, true, true),
        ("GPKG", &["gpkg"][..], true, true, true, true),
        (
            "GeoParquet",
            &["parquet", "geoparquet"][..],
            false,
            true,
            true,
            true,
        ),
        ("KML", &["kml", "kmz"][..], false, true, true, true),
        ("GPX", &["gpx"][..], false, false, false, true),
    ]
    .into_iter()
    .map(
        |(name, ext, layers, collections, polygons, web_export)| Format {
            name: name.into(),
            extensions: ext.iter().map(|s| (*s).into()).collect(),
            read: true,
            write: true,
            layers,
            collections,
            polygons,
            web_export,
            native: true,
        },
    )
    .collect::<Vec<_>>();
    #[cfg(feature = "gdal-system")]
    formats.extend(
        crate::gdal_backend::formats()
            .into_iter()
            .filter(|f| !formats_native_name(&f.name)),
    );
    formats.sort_by(|a, b| a.name.cmp(&b.name));
    formats
}
#[cfg(feature = "gdal-system")]
fn formats_native_name(name: &str) -> bool {
    matches!(
        name,
        "GeoJSON" | "ESRI Shapefile" | "FlatGeobuf" | "GPKG" | "Parquet" | "KML" | "LIBKML" | "GPX"
    )
}
pub fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}
pub fn for_path(path: &Path) -> Option<Format> {
    let ext = extension(path);
    let all = formats();
    all.iter()
        .find(|f| f.native && f.extensions.contains(&ext))
        // GPSBabel also lists Garmin .gdb; prefer drivers whose primary extension matches.
        .or_else(|| all.iter().find(|f| f.extensions.first() == Some(&ext)))
        .or_else(|| all.iter().find(|f| f.extensions.contains(&ext)))
        .cloned()
}
/// # Errors
/// Returns an error for an unknown format or a format without write support.
pub fn writable(path: &Path, explicit: Option<&str>) -> Result<Format> {
    let format = if let Some(name) = explicit {
        formats().into_iter().find(|f| {
            f.name.eq_ignore_ascii_case(name)
                || (name.eq_ignore_ascii_case("Parquet") && f.name == "GeoParquet")
                || (name.eq_ignore_ascii_case("LIBKML") && f.name == "KML")
        })
    } else {
        for_path(path)
    }
    .context("unknown output format; use a supported extension or driver")?;
    if !format.write {
        bail!("format {} cannot create vector datasets", format.name);
    }
    Ok(format)
}
pub fn project_extension(ext: &str) -> bool {
    ext == "zip"
        || formats()
            .iter()
            .any(|f| f.native && f.read && f.extensions.iter().any(|e| e == ext))
}
