use crate::{crs::Crs, registry};
use anyhow::{Context, Result, bail, ensure};
use geo::Geometry;
use serde_json::{Map, Value};
use std::{collections::HashSet, path::Path};
use topology_checker_core::Layer;

#[derive(Debug)]
pub struct ReadLayer {
    pub layer: Layer,
    pub srs: Option<Crs>,
    pub warnings: Vec<String>,
    pub selector: String,
    pub attributes: Vec<Map<String, Value>>,
}
#[derive(Debug)]
pub(crate) struct RawFeature {
    pub id: Option<i64>,
    pub geometry: Option<Geometry>,
    pub properties: Map<String, Value>,
    pub zm: bool,
}
#[derive(Debug)]
pub(crate) struct RawLayer {
    pub name: String,
    pub srs: Option<Crs>,
    pub features: Vec<RawFeature>,
}
/// Read vector layers, retaining source identities independently of attributes.
/// # Errors
/// Returns an error for malformed input, unknown layers, duplicate IDs or unsupported formats.
pub fn read(path: &Path, selector: Option<&str>, alias: &str) -> Result<Vec<ReadLayer>> {
    read_with_attributes(path, selector, alias, false)
}
/// Read supported local vector files with optional aligned inspection attributes.
/// # Errors
/// Returns an error for malformed input, unknown layers, duplicate IDs or unsupported formats.
pub fn read_with_attributes(
    path: &Path,
    selector: Option<&str>,
    alias: &str,
    attributes: bool,
) -> Result<Vec<ReadLayer>> {
    let format = registry::for_path(path).context("unsupported vector format")?;
    if !format.native {
        #[cfg(feature = "gdal-system")]
        return crate::gdal_read::read_with_attributes(path, selector, alias, attributes);
        #[cfg(not(feature = "gdal-system"))]
        bail!("format requires the optional GDAL backend");
    }
    let raw = match format.name.as_str() {
        "GeoJSON" => crate::native::read_geojson(path)?,
        "GPKG" => crate::gpkg::read(path)?,
        "ESRI Shapefile" => crate::native::read_shape(path)?,
        "FlatGeobuf" => crate::native::read_fgb(path)?,
        "GeoParquet" => crate::parquet::read(path)?,
        "KML" => crate::xml::read_kml(path)?,
        // Every other native format is GPX.
        _ => crate::xml::read_gpx(path)?,
    };
    let mut raw = raw
        .into_iter()
        .filter(|l| selector.is_none_or(|s| s == l.name))
        .collect::<Vec<_>>();
    ensure!(
        !raw.is_empty(),
        "no vector layers match selection in {}",
        path.display()
    );
    let multiple = raw.len() > 1;
    raw.drain(..)
        .map(|l| {
            let label = if multiple {
                format!("{alias}:{}", l.name)
            } else {
                alias.into()
            };
            finish(l, label, attributes)
        })
        .collect()
}
pub(crate) fn finish(raw: RawLayer, label: String, attributes: bool) -> Result<ReadLayer> {
    let mut used = HashSet::new();
    // Reserve even skipped source IDs so fallback identities do not depend on geometry filtering.
    for f in &raw.features {
        if let Some(id) = f.id {
            ensure!(used.insert(id), "layer {label}: duplicate feature ID {id}");
        }
    }
    let mut features = Vec::new();
    let mut values = Vec::new();
    let mut nulls = 0;
    let mut empties = 0;
    let mut zm = 0;
    let mut missing = 0;
    let mut candidate = -1_i64;
    for f in raw.features {
        let Some(geometry) = f.geometry else {
            nulls += 1;
            continue;
        };
        if crate::codec::empty(&geometry) {
            empties += 1;
            continue;
        }
        let geometry = crate::codec::without_empty_members(geometry);
        if f.zm {
            zm += 1;
        }
        let id = if let Some(id) = f.id {
            id
        } else {
            while used.contains(&candidate) {
                candidate = candidate
                    .checked_sub(1)
                    .context("no available fallback FID")?;
            }
            let id = candidate;
            used.insert(id);
            missing += 1;
            id
        };
        features.push((id, geometry));
        if attributes {
            values.push(
                f.properties
                    .into_iter()
                    .map(|(k, v)| (k, browser_json(v)))
                    .collect(),
            );
        }
    }
    let mut warnings = Vec::new();
    for (n, message) in [
        (nulls, "skipped {} null geometries"),
        (empties, "skipped {} empty geometries"),
        (zm, "dropped Z/M values from {} geometries"),
        (missing, "assigned negative FIDs to {} features without IDs"),
    ] {
        if n > 0 {
            warnings.push(format!(
                "layer {label}: {}",
                message.replace("{}", &n.to_string())
            ));
        }
    }
    if raw.srs.is_none() {
        warnings.push(format!("layer {label}: missing CRS"));
    }
    Ok(ReadLayer {
        layer: Layer::from_features(label, features),
        srs: raw.srs,
        warnings,
        selector: raw.name,
        attributes: values,
    })
}
pub(crate) fn browser_json(value: Value) -> Value {
    match value {
        Value::Number(n)
            if n.as_i64()
                .is_some_and(|n| n.unsigned_abs() > 9_007_199_254_740_991)
                || n.as_u64().is_some_and(|n| n > 9_007_199_254_740_991) =>
        {
            Value::String(n.to_string())
        }
        Value::Array(v) => Value::Array(v.into_iter().map(browser_json).collect()),
        Value::Object(v) => {
            Value::Object(v.into_iter().map(|(k, v)| (k, browser_json(v))).collect())
        }
        _ => value,
    }
}
/// # Errors
/// Returns an error if any known input CRSs differ, even when other layers lack a CRS.
pub fn common_crs(layers: &[ReadLayer]) -> Result<Option<Crs>> {
    let mut first = None;
    let mut missing = false;
    for layer in layers {
        if let Some(srs) = &layer.srs {
            if let Some(known) = first {
                if known != srs {
                    bail!("input layers have different CRSs; reprojection is required");
                }
            } else {
                first = Some(srs);
            }
        } else {
            missing = true;
        }
    }
    Ok(if missing { None } else { first.cloned() })
}
pub fn default_tolerance(srs: Option<&Crs>) -> f64 {
    match srs {
        Some(s) if s.is_geographic() => 1e-8,
        Some(s) if s.is_projected() && s.linear_units().is_finite() && s.linear_units() > 0. => {
            0.001 / s.linear_units()
        }
        _ => 0.,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layer(epsg: Option<u32>) -> ReadLayer {
        ReadLayer {
            layer: Layer::from_geometries("empty", []),
            srs: epsg.map(|code| crate::crs::parse(&format!("EPSG:{code}")).unwrap()),
            warnings: Vec::new(),
            selector: "empty".into(),
            attributes: Vec::new(),
        }
    }
    #[test]
    fn missing_crs_does_not_hide_mismatches_between_known_layers() {
        for layers in [
            [layer(None), layer(Some(4326)), layer(Some(3857))],
            [layer(Some(4326)), layer(None), layer(Some(3857))],
            [layer(Some(4326)), layer(Some(3857)), layer(None)],
        ] {
            assert!(
                common_crs(&layers)
                    .unwrap_err()
                    .to_string()
                    .contains("different CRSs")
            );
        }
        assert!(
            common_crs(&[layer(None), layer(Some(4326)), layer(Some(4326))])
                .unwrap()
                .is_none()
        );
        assert_eq!(
            common_crs(&[layer(Some(3857)), layer(Some(3857))])
                .unwrap()
                .unwrap()
                .epsg()
                .unwrap(),
            3857
        );
        assert!(common_crs(&[]).unwrap().is_none());
    }
}
