pub use crate::registry::Format as DriverInfo;
use crate::{Crs, codec, registry};
use anyhow::{Context, Result, ensure};
use geo::{Geometry, MultiLineString, MultiPoint, MultiPolygon};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};
use topology_checker_core::{FeatureRef, Report, Violation};

pub fn drivers(read: bool, write: bool) -> Vec<DriverInfo> {
    registry::formats()
        .into_iter()
        .filter(|f| (!read || f.read) && (!write || f.write))
        .collect()
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Integer,
    Real,
    Boolean,
    Text,
}
#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub kind: FieldType,
}
#[derive(Debug, Clone)]
pub struct Record {
    pub id: Option<i64>,
    pub geometry: Option<Geometry>,
    pub properties: Map<String, Value>,
}
#[derive(Debug, Clone)]
pub struct DatasetLayer {
    pub name: String,
    pub srs: Option<Crs>,
    pub geometry_type: Option<String>,
    pub fields: Vec<Field>,
    pub records: Vec<Record>,
}
impl DatasetLayer {
    /// Construct a layer and infer typed attributes, including null-only columns.
    pub fn new(name: impl Into<String>, srs: Option<Crs>, records: Vec<Record>) -> Self {
        let mut fields = BTreeMap::new();
        for record in &records {
            for (name, v) in &record.properties {
                let kind = match v {
                    Value::Bool(_) => FieldType::Boolean,
                    Value::Number(n) if n.is_i64() => FieldType::Integer,
                    Value::Number(_) => FieldType::Real,
                    Value::Null => continue,
                    _ => FieldType::Text,
                };
                fields
                    .entry(name.clone())
                    .and_modify(|k| {
                        if *k != kind {
                            *k = if matches!(
                                (*k, kind),
                                (FieldType::Integer, FieldType::Real)
                                    | (FieldType::Real, FieldType::Integer)
                            ) {
                                FieldType::Real
                            } else {
                                FieldType::Text
                            };
                        }
                    })
                    .or_insert(kind);
            }
        }
        for record in &records {
            for name in record.properties.keys() {
                fields.entry(name.clone()).or_insert(FieldType::Text);
            }
        }
        Self {
            name: name.into(),
            srs,
            geometry_type: None,
            fields: fields
                .into_iter()
                .map(|(name, kind)| Field { name, kind })
                .collect(),
            records,
        }
    }
}
/// Write complete records and geometries without flattening collections.
/// Native formats always take precedence, even in GDAL builds.
/// # Errors
/// Returns an error for incompatible geometry, duplicate IDs, existing outputs or write failures.
pub fn dataset(
    path: &Path,
    explicit: Option<&str>,
    overwrite: bool,
    layers: &[DatasetLayer],
) -> Result<()> {
    let format = registry::writable(path, explicit)?;
    let mut prepared = layers.to_vec();
    for layer in &mut prepared {
        let mut used = layer
            .records
            .iter()
            .filter_map(|r| r.id)
            .collect::<HashSet<_>>();
        let mut next = -1_i64;
        for record in &mut layer.records {
            if format.native && record.id.is_none() {
                while used.contains(&next) {
                    next = next.checked_sub(1).context("no available output FID")?;
                }
                record.id = Some(next);
                used.insert(next);
            }
        }
    }
    let layers = prepared.as_slice();
    ensure!(!layers.is_empty(), "dataset needs at least one layer");
    ensure!(
        format.layers || layers.len() == 1,
        "{} supports one layer per file",
        format.name
    );
    let mut names = HashSet::new();
    for l in layers {
        ensure!(
            names.insert(l.name.to_ascii_lowercase()),
            "duplicate dataset layer name"
        );
        let mut ids = HashSet::new();
        let mut fields = HashSet::new();
        for f in &l.fields {
            ensure!(
                fields.insert(f.name.to_ascii_lowercase()),
                "duplicate attribute name"
            );
        }
        for r in &l.records {
            if let Some(id) = r.id {
                ensure!(ids.insert(id), "duplicate feature ID {id}");
            }
            if let Some(g) = &r.geometry {
                use geo::CoordsIter;
                ensure!(
                    codec::empty(g) || g.coords_iter().all(|c| c.x.is_finite() && c.y.is_finite()),
                    "non-finite output geometry"
                );
                ensure!(
                    format.collections || !matches!(g, Geometry::GeometryCollection(_)),
                    "{} cannot preserve geometry collections",
                    format.name
                );
            }
        }
    }
    ensure!(
        !path.exists() || overwrite,
        "output {} already exists; use --overwrite",
        path.display()
    );
    ensure!(
        !path.is_dir() || format.name == "OpenFileGDB",
        "output {} is a directory",
        path.display()
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let staging = tempfile::Builder::new()
        .prefix(".topology-output-")
        .tempdir_in(parent)?;
    let temporary = staging
        .path()
        .join(path.file_name().context("output filename required")?);
    match format.name.as_str() {
        "GPKG" => crate::gpkg::write(&temporary, layers)?,
        "GeoJSON" => crate::native::write_geojson(&temporary, &layers[0])?,
        "ESRI Shapefile" => crate::native::write_shape(&temporary, &layers[0])?,
        "FlatGeobuf" => crate::native::write_fgb(&temporary, &layers[0])?,
        "GeoParquet" => crate::parquet::write(&temporary, &layers[0])?,
        "KML" => crate::xml::write_kml(&temporary, &layers[0])?,
        "GPX" => crate::xml::write_gpx(&temporary, &layers[0])?,
        _ => {
            #[cfg(feature = "gdal-system")]
            crate::gdal_backend::write(&temporary, &format.name, layers)?;
            #[cfg(not(feature = "gdal-system"))]
            anyhow::bail!("output requires optional GDAL backend");
        }
    }
    let files = std::fs::read_dir(staging.path())?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|p| {
            !matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("gpkg-wal" | "gpkg-shm")
            )
        })
        .collect::<Vec<_>>();
    for file in &files {
        let target = parent.join(file.file_name().unwrap());
        ensure!(
            !target.exists() || overwrite,
            "output {} already exists; use --overwrite",
            target.display()
        );
    }
    for file in files {
        let target = parent.join(file.file_name().unwrap());
        if target.is_dir() && format.name == "OpenFileGDB" {
            let backup = staging.path().join("previous-dataset");
            std::fs::rename(&target, &backup)?;
            if let Err(error) = std::fs::rename(&file, &target) {
                std::fs::rename(&backup, &target)?;
                return Err(error.into());
            }
            continue;
        }
        std::fs::rename(&file, &target)
            .with_context(|| format!("replacing {}", target.display()))?;
    }
    Ok(())
}
/// Violation attribute columns shared by every report layout, in output order.
const REPORT_FIELDS: [(&str, FieldType); 9] = [
    ("violation_id", FieldType::Integer),
    ("rule", FieldType::Text),
    ("error_type", FieldType::Text),
    ("origin_layer", FieldType::Text),
    ("origin_fid", FieldType::Integer),
    ("origin_part", FieldType::Integer),
    ("destination_layer", FieldType::Text),
    ("destination_fid", FieldType::Integer),
    ("destination_part", FieldType::Integer),
];
/// Shapefile names for report columns longer than its 10-character limit.
const SHAPEFILE_NAMES: [(&str, &str); 6] = [
    ("violation_id", "viol_id"),
    ("origin_layer", "orig_layer"),
    ("origin_part", "orig_part"),
    ("destination_layer", "dest_layer"),
    ("destination_fid", "dest_fid"),
    ("destination_part", "dest_part"),
];

/// How violation geometries are arranged into report layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportLayout {
    /// `errors_point`, `errors_line` and `errors_polygon` layers with multi geometries.
    /// A mixed violation contributes a row to each family, sharing its `violation_id`.
    /// Formats with one layer per file write one file per family. A valid report
    /// writes nothing.
    ByFamily,
    /// One `violations` layer with each violation's complete geometry, written even
    /// when the report is valid. The format must support geometry collections when a
    /// violation mixes dimensions.
    Complete,
}

fn report_fields() -> Vec<Field> {
    REPORT_FIELDS
        .into_iter()
        .map(|(name, kind)| Field {
            name: name.into(),
            kind,
        })
        .collect()
}
fn shorten_for_shapefile(layer: &mut DatasetLayer) {
    for (long, short) in SHAPEFILE_NAMES {
        for r in &mut layer.records {
            if let Some(v) = r.properties.remove(long) {
                r.properties.insert(short.into(), v);
            }
        }
        for f in &mut layer.fields {
            if f.name == long {
                f.name = short.into();
            }
        }
    }
}
/// Attribute every violation in report order with its rule and source features.
fn violation_records(report: &Report) -> Vec<(&Violation, Map<String, Value>)> {
    let mut records = Vec::with_capacity(report.violation_count());
    for rule in &report.rules {
        for v in &rule.violations {
            let source = |r: Option<FeatureRef>, role: &str, properties: &mut Map<_, _>| {
                properties.insert(
                    format!("{role}_layer"),
                    json!(r.and_then(|r| rule.layer_name(r.layer))),
                );
                properties.insert(format!("{role}_fid"), json!(r.map(|r| r.fid)));
                properties.insert(format!("{role}_part"), json!(r.and_then(|r| r.part)));
            };
            let mut properties = Map::new();
            properties.insert("violation_id".into(), json!(records.len()));
            properties.insert("rule".into(), json!(v.rule.name()));
            properties.insert("error_type".into(), json!(v.kind.name()));
            source(v.origin, "origin", &mut properties);
            source(v.destination, "destination", &mut properties);
            records.push((v, properties));
        }
    }
    records
}
fn group(
    g: &Geometry,
    properties: Map<String, Value>,
    groups: &mut BTreeMap<&'static str, Vec<Record>>,
) {
    let (kind, geometry) = match g {
        Geometry::Point(p) => ("point", MultiPoint(vec![*p]).into()),
        Geometry::MultiPoint(_) => ("point", g.clone()),
        Geometry::Line(l) => ("line", MultiLineString(vec![(*l).into()]).into()),
        Geometry::LineString(l) => ("line", MultiLineString(vec![l.clone()]).into()),
        Geometry::MultiLineString(_) => ("line", g.clone()),
        Geometry::Polygon(p) => ("polygon", MultiPolygon(vec![p.clone()]).into()),
        Geometry::MultiPolygon(_) => ("polygon", g.clone()),
        Geometry::Rect(r) => {
            group(&r.to_polygon().into(), properties, groups);
            return;
        }
        Geometry::Triangle(t) => {
            group(&t.to_polygon().into(), properties, groups);
            return;
        }
        Geometry::GeometryCollection(c) => {
            for part in c {
                group(part, properties.clone(), groups);
            }
            return;
        }
    };
    groups.entry(kind).or_default().push(Record {
        id: None,
        geometry: Some(geometry),
        properties,
    });
}
/// Write violations with the shared report columns, returning the written paths.
/// Layer names are resolved from the report, so no topology is required.
/// # Errors
/// Returns an error for incompatible output formats, existing outputs or write failures.
pub fn report(
    path: &Path,
    explicit: Option<&str>,
    overwrite: bool,
    srs: Option<&Crs>,
    report: &Report,
    layout: ReportLayout,
) -> Result<Vec<PathBuf>> {
    match layout {
        ReportLayout::ByFamily => {
            if report.is_valid() {
                return Ok(Vec::new());
            }
            let mut groups = BTreeMap::new();
            for (v, properties) in violation_records(report) {
                group(&v.geometry, properties, &mut groups);
            }
            grouped(path, explicit, overwrite, srs, groups, "errors")
        }
        ReportLayout::Complete => {
            let records = violation_records(report)
                .into_iter()
                .zip(1..)
                .map(|((v, properties), id)| Record {
                    id: Some(id),
                    geometry: Some(v.geometry.clone()),
                    properties,
                })
                .collect();
            let mut layer = DatasetLayer::new("violations", srs.cloned(), records);
            layer.fields = report_fields();
            if registry::writable(path, explicit)?.name == "ESRI Shapefile" {
                shorten_for_shapefile(&mut layer);
            }
            dataset(path, explicit, overwrite, &[layer])?;
            Ok(vec![path.to_path_buf()])
        }
    }
}
/// Write utility output, including a readable empty line layer.
/// # Errors
/// Returns an error for incompatible output formats, existing outputs or write failures.
pub fn geometries(
    path: &Path,
    explicit: Option<&str>,
    overwrite: bool,
    srs: Option<&Crs>,
    geometries: &[Geometry],
    name: &str,
) -> Result<Vec<PathBuf>> {
    let mut groups = BTreeMap::new();
    for g in geometries {
        group(g, Map::new(), &mut groups);
    }
    if groups.is_empty() {
        groups.insert("line", Vec::new());
    }
    grouped(path, explicit, overwrite, srs, groups, name)
}
fn grouped(
    path: &Path,
    explicit: Option<&str>,
    overwrite: bool,
    srs: Option<&Crs>,
    groups: BTreeMap<&str, Vec<Record>>,
    prefix: &str,
) -> Result<Vec<PathBuf>> {
    let format = registry::writable(path, explicit)?;
    let mut layers = Vec::new();
    for (kind, mut records) in groups {
        // Utility and report records have newly created, sequential identities.
        // Keep their iteration order in integer-primary-key datasets.
        for (i, record) in records.iter_mut().enumerate() {
            record.id = Some(i64::try_from(i)? + 1);
        }
        let mut layer = DatasetLayer::new(format!("{prefix}_{kind}"), srs.cloned(), records);
        layer.geometry_type = Some(
            match kind {
                "point" => "MULTIPOINT",
                "line" => "MULTILINESTRING",
                _ => "MULTIPOLYGON",
            }
            .into(),
        );
        if prefix == "errors" {
            layer.fields = report_fields();
            if format.name == "ESRI Shapefile" {
                shorten_for_shapefile(&mut layer);
            }
        }
        layers.push(layer);
    }
    let outputs = if format.layers || layers.len() == 1 {
        vec![(path.to_path_buf(), layers)]
    } else {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .context("invalid output filename")?;
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .context("output extension required for per-geometry files")?;
        layers
            .into_iter()
            .map(|l| {
                let suffix = l.name.strip_prefix(&format!("{prefix}_")).unwrap();
                (
                    path.with_file_name(format!("{stem}_{suffix}.{ext}")),
                    vec![l],
                )
            })
            .collect()
    };
    for (p, _) in &outputs {
        ensure!(
            !p.exists() || overwrite,
            "output {} already exists; use --overwrite",
            p.display()
        );
    }
    let mut written = Vec::new();
    for (p, layers) in outputs {
        dataset(&p, explicit, overwrite, &layers)?;
        written.push(p);
    }
    Ok(written)
}
