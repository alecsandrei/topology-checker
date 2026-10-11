use anyhow::{Context, Result, bail};
use gdal::{
    Dataset, Driver, DriverManager, Metadata,
    spatial_ref::SpatialRef,
    vector::{Feature, LayerAccess, LayerOptions, OGRFieldType, ToGdal},
};
use geo::{Geometry, MultiLineString, MultiPoint, MultiPolygon};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use topology_checker_core::{Report, Topology, TopologyError};

#[derive(Debug, Serialize)]
pub struct DriverInfo {
    pub name: String,
    pub extensions: Vec<String>,
    pub read: bool,
    pub write: bool,
}
pub fn drivers(read: bool, write: bool) -> Vec<DriverInfo> {
    let mut all = (0..DriverManager::count())
        .filter_map(|i| DriverManager::get_driver(i).ok())
        .filter(|d| capability(d, "DCAP_VECTOR"))
        .map(|d| DriverInfo {
            name: d.short_name(),
            extensions: extensions(&d),
            read: capability(&d, "DCAP_OPEN"),
            write: capability(&d, "DCAP_CREATE"),
        })
        .filter(|d| (!read || d.read) && (!write || d.write))
        .collect::<Vec<_>>();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}
fn capability(d: &Driver, key: &str) -> bool {
    d.metadata_item(key, "").is_some_and(|v| v == "YES")
}
fn extensions(d: &Driver) -> Vec<String> {
    d.metadata_item("DMD_EXTENSIONS", "")
        .filter(|s| !s.is_empty())
        .or_else(|| d.metadata_item("DMD_EXTENSION", ""))
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect()
}
fn driver(path: &Path, explicit: Option<&str>) -> Result<Driver> {
    let name = if let Some(name) = explicit {
        name.to_string()
    } else {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .context("output has no extension; specify a driver")?
            .to_lowercase();
        let matches = drivers(false, true)
            .into_iter()
            .filter(|d| d.extensions.contains(&ext))
            .map(|d| d.name)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            bail!(
                "extension .{ext} maps to {} writable drivers; specify a driver: {}",
                matches.len(),
                matches.join(", ")
            );
        }
        matches[0].clone()
    };
    let d = DriverManager::get_driver_by_name(&name)
        .with_context(|| format!("unknown GDAL driver {name}"))?;
    if !capability(&d, "DCAP_VECTOR") || !capability(&d, "DCAP_CREATE") {
        bail!("driver {name} cannot create vector datasets");
    }
    Ok(d)
}
#[derive(Clone)]
struct Attributes {
    rule: &'static str,
    error_type: &'static str,
    origin_layer: Option<String>,
    origin_fid: Option<i64>,
    destination_layer: Option<String>,
    destination_fid: Option<i64>,
}

#[derive(Clone)]
struct Record {
    geometry: Geometry,
    fields: Option<Attributes>,
}
fn group_geometry(
    g: &Geometry,
    fields: Option<Attributes>,
    groups: &mut BTreeMap<&'static str, Vec<Record>>,
) {
    let (kind, geometry) = match g {
        Geometry::Point(p) => ("point", Geometry::MultiPoint(MultiPoint(vec![*p]))),
        Geometry::MultiPoint(p) => ("point", p.clone().into()),
        Geometry::Line(l) => (
            "line",
            Geometry::MultiLineString(MultiLineString(vec![(*l).into()])),
        ),
        Geometry::LineString(l) => (
            "line",
            Geometry::MultiLineString(MultiLineString(vec![l.clone()])),
        ),
        Geometry::MultiLineString(l) => ("line", l.clone().into()),
        Geometry::Polygon(p) => (
            "polygon",
            Geometry::MultiPolygon(MultiPolygon(vec![p.clone()])),
        ),
        Geometry::MultiPolygon(p) => ("polygon", p.clone().into()),
        Geometry::Rect(r) => {
            group_geometry(&r.to_polygon().into(), fields, groups);
            return;
        }
        Geometry::Triangle(t) => {
            group_geometry(&t.to_polygon().into(), fields, groups);
            return;
        }
        Geometry::GeometryCollection(c) => {
            for part in c {
                group_geometry(part, fields.clone(), groups);
            }
            return;
        }
    };
    groups
        .entry(kind)
        .or_default()
        .push(Record { geometry, fields });
}
fn fields(e: &TopologyError, t: &Topology) -> Result<Attributes> {
    Ok(Attributes {
        rule: e.rule.name(),
        error_type: e.error_type.name(),
        origin_layer: e
            .origin
            .map(|r| t.layer(r.layer).map(|l| l.name().to_string()))
            .transpose()?,
        origin_fid: e.origin.map(|r| r.fid),
        destination_layer: e
            .destination
            .map(|r| t.layer(r.layer).map(|l| l.name().to_string()))
            .transpose()?,
        destination_fid: e.destination.map(|r| r.fid),
    })
}
/// Write CLI violations, grouping geometry families for driver compatibility.
///
/// # Errors
/// Returns an error for unavailable drivers, conflicting outputs or GDAL write failures.
pub fn report(
    path: &Path,
    explicit: Option<&str>,
    overwrite: bool,
    srs: Option<&SpatialRef>,
    report: &Report,
    t: &Topology,
) -> Result<Vec<PathBuf>> {
    if report.is_valid() {
        return Ok(Vec::new());
    }
    let mut groups = BTreeMap::new();
    for error in report.errors() {
        group_geometry(&error.geometry, Some(fields(error, t)?), &mut groups);
    }
    write(path, explicit, overwrite, srs, groups, "errors")
}
/// Write utility geometries using the requested or inferred vector driver.
///
/// # Errors
/// Returns an error for invalid drivers, existing outputs or GDAL write failures.
pub fn geometries(
    path: &Path,
    explicit: Option<&str>,
    overwrite: bool,
    srs: Option<&SpatialRef>,
    geometries: &[Geometry],
    name: &str,
) -> Result<Vec<PathBuf>> {
    let mut groups = BTreeMap::new();
    for g in geometries {
        group_geometry(g, None, &mut groups);
    }
    // Empty utilities still produce a readable, empty line layer.
    if groups.is_empty() {
        groups.insert("line", Vec::new());
    }
    write(path, explicit, overwrite, srs, groups, name)
}
fn write(
    path: &Path,
    explicit: Option<&str>,
    overwrite: bool,
    srs: Option<&SpatialRef>,
    groups: BTreeMap<&'static str, Vec<Record>>,
    name: &str,
) -> Result<Vec<PathBuf>> {
    let drv = driver(path, explicit)?;
    let multiple = capability(&drv, "DCAP_MULTIPLE_VECTOR_LAYERS");
    let mut outputs: Vec<(PathBuf, BTreeMap<&str, Vec<Record>>)> = Vec::new();
    if multiple || groups.len() == 1 {
        outputs.push((path.to_path_buf(), groups));
    } else {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .context("invalid output filename")?;
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .context("output extension required for per-geometry files")?;
        for (kind, records) in groups {
            outputs.push((
                path.with_file_name(format!("{stem}_{kind}.{ext}")),
                BTreeMap::from([(kind, records)]),
            ));
        }
    }
    // Check every output before touching any of them.
    for (path, _) in &outputs {
        if path.exists() && !overwrite {
            bail!("output {} already exists; use --overwrite", path.display());
        }
        if path.is_dir() && drv.short_name() != "OpenFileGDB" {
            bail!("output {} is a directory", path.display());
        }
    }
    let mut written = Vec::new();
    for (path, groups) in outputs {
        if path.exists() {
            drv.delete(&path)
                .with_context(|| format!("replacing {}", path.display()))?;
        }
        let mut ds = drv
            .create_vector_only(&path)
            .with_context(|| format!("creating {}", path.display()))?;
        let result = if ds.has_capability(gdal::DatasetCapability::TRANSACTIONS) {
            let mut transaction = ds.start_transaction()?;
            populate(&mut transaction, groups, name, srs).and_then(|()| Ok(transaction.commit()?))
        } else {
            populate(&mut ds, groups, name, srs)
        };
        result.with_context(|| format!("writing {}", path.display()))?;
        ds.flush_cache()?;
        drop(ds);
        written.push(path);
    }
    Ok(written)
}
fn populate(
    ds: &mut Dataset,
    groups: BTreeMap<&str, Vec<Record>>,
    prefix: &str,
    srs: Option<&SpatialRef>,
) -> Result<()> {
    use gdal_sys::OGRwkbGeometryType::*;
    let short_fields = ds.driver().short_name() == "ESRI Shapefile";
    for (kind, records) in groups {
        let name = format!("{prefix}_{kind}");
        let ty = match kind {
            "point" => wkbMultiPoint,
            "line" => wkbMultiLineString,
            _ => wkbMultiPolygon,
        };
        let layer = ds.create_layer(LayerOptions {
            name: &name,
            srs,
            ty,
            ..Default::default()
        })?;
        let has_fields = prefix == "errors";
        if has_fields {
            let names = if short_fields {
                [
                    "rule",
                    "error_type",
                    "orig_layer",
                    "origin_fid",
                    "dest_layer",
                    "dest_fid",
                ]
            } else {
                [
                    "rule",
                    "error_type",
                    "origin_layer",
                    "origin_fid",
                    "destination_layer",
                    "destination_fid",
                ]
            };
            layer.create_defn_fields(
                &names
                    .into_iter()
                    .zip([
                        OGRFieldType::OFTString,
                        OGRFieldType::OFTString,
                        OGRFieldType::OFTString,
                        OGRFieldType::OFTInteger64,
                        OGRFieldType::OFTString,
                        OGRFieldType::OFTInteger64,
                    ])
                    .collect::<Vec<_>>(),
            )?;
        }
        for record in records {
            let mut feature = Feature::new(layer.defn())?;
            feature.set_geometry(record.geometry.to_gdal()?)?;
            if let Some(fields) = record.fields {
                feature.set_field_string(0, fields.rule)?;
                feature.set_field_string(1, fields.error_type)?;
                for (i, value) in [(2, fields.origin_layer), (4, fields.destination_layer)] {
                    if let Some(value) = value {
                        feature.set_field_string(i, &value)?;
                    } else {
                        feature.set_field_null(i)?;
                    }
                }
                for (i, value) in [(3, fields.origin_fid), (5, fields.destination_fid)] {
                    if let Some(value) = value {
                        feature.set_field_integer64(i, value)?;
                    } else {
                        feature.set_field_null(i)?;
                    }
                }
            }
            feature.create(&layer)?;
        }
    }
    Ok(())
}
