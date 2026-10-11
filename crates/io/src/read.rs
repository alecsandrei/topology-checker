use anyhow::{Context, Result, bail};
use gdal::{
    Dataset, DatasetOptions, GdalOpenFlags,
    spatial_ref::SpatialRef,
    vector::{LayerAccess, geometry_type_has_m, geometry_type_has_z},
};
use geo::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};
use std::{collections::HashSet, path::Path};
use topology_checker_core::Layer;

#[derive(Debug)]
pub struct ReadLayer {
    pub layer: Layer,
    pub srs: Option<SpatialRef>,
    pub warnings: Vec<String>,
    /// Source dataset layer name, independent of the display alias.
    pub selector: String,
    /// Attributes aligned with retained features; empty when not requested.
    pub attributes: Vec<serde_json::Map<String, serde_json::Value>>,
}

/// Read vector layers with the CLI's existing ID and geometry handling.
///
/// # Errors
/// Returns an error for inaccessible datasets, missing layers or unsupported geometries.
pub fn read(path: &Path, selector: Option<&str>, alias: &str) -> Result<Vec<ReadLayer>> {
    read_with_attributes(path, selector, alias, false)
}

/// Read layers with optional inspection attributes and native GeoJSON identities.
/// Inspection restricts GDAL drivers to the advertised local file format.
///
/// # Errors
/// Returns an error for invalid files, selections, geometry types or native JSON.
pub fn read_with_attributes(
    path: &Path,
    selector: Option<&str>,
    alias: &str,
    attributes: bool,
) -> Result<Vec<ReadLayer>> {
    let driver = match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "geojson" | "json" => "GeoJSON",
        "gpkg" => "GPKG",
        "fgb" => "FlatGeobuf",
        "shp" => "ESRI Shapefile",
        _ if attributes => bail!("unsupported inspection format"),
        _ => "",
    };
    let allowed = [driver];
    let native_options = ["NATIVE_DATA=YES", "DATE_AS_STRING=YES"];
    let dataset = Dataset::open_ex(
        path,
        DatasetOptions {
            open_flags: GdalOpenFlags::GDAL_OF_VECTOR,
            allowed_drivers: attributes.then_some(&allowed),
            open_options: (attributes && driver == "GeoJSON").then_some(&native_options),
            ..Default::default()
        },
    )
    .with_context(|| format!("opening {}", path.display()))?;
    let names = if let Some(name) = selector {
        vec![name.to_string()]
    } else {
        dataset.layers().map(|l| l.name()).collect::<Vec<_>>()
    };
    if names.is_empty() {
        bail!("{} contains no vector layers", path.display());
    }
    let multiple = names.len() > 1;
    names
        .into_iter()
        .map(|name| {
            let mut layer = dataset
                .layer_by_name(&name)
                .with_context(|| format!("layer {name} in {}", path.display()))?;
            let srs = layer.spatial_ref();
            let mut warnings = Vec::new();
            let label = if multiple {
                format!("{alias}:{name}")
            } else {
                alias.to_string()
            };
            let mut features = Vec::new();
            let mut values = Vec::new();
            let mut nulls = 0;
            let mut empties = 0;
            let mut zm = 0;
            let mut missing = Vec::new();
            for (ordinal, feature) in layer.features().enumerate() {
                // SAFETY: the iterator owns a live feature for this entire call. GDAL FIDs
                // are signed; gdal::Feature::fid treats every negative value as absent,
                // whereas only OGRNullFID (-1) denotes an unassigned ID.
                let gdal_fid = unsafe { gdal_sys::OGR_F_GetFID(feature.c_feature()) };
                let native = if attributes && driver == "GeoJSON" {
                    // SAFETY: GDAL owns the native data for this live feature. Copy
                    // and parse it before the feature advances or is dropped.
                    let pointer = unsafe { gdal_sys::OGR_F_GetNativeData(feature.c_feature()) };
                    if pointer.is_null() {
                        None
                    } else {
                        // SAFETY: the non-null pointer is a null-terminated string
                        // owned by the live GDAL feature.
                        let native = unsafe { std::ffi::CStr::from_ptr(pointer) };
                        Some(serde_json::from_slice::<serde_json::Value>(
                            native.to_bytes(),
                        )?)
                    }
                } else {
                    None
                };
                let native_fid = native.as_ref().and_then(|n| n.get("id")).and_then(|id| {
                    id.as_i64()
                        .or_else(|| id.as_str().and_then(|s| s.parse().ok()))
                });
                let has_id = if native.is_some() {
                    native_fid.is_some()
                } else {
                    gdal_fid != -1
                };
                let raw_fid = native_fid.unwrap_or(gdal_fid);
                let fid = if has_id {
                    raw_fid
                } else {
                    -(ordinal as i64) - 1
                };
                // SAFETY: feature owns a live OGR feature. gdal 0.19's geometry()
                // can wrap a null pointer when a declared geometry field is null.
                // Check it before invoking any geometry operations.
                if unsafe { gdal_sys::OGR_F_GetGeometryRef(feature.c_feature()) }.is_null() {
                    nulls += 1;
                    continue;
                }
                let Some(g) = feature.geometry() else {
                    nulls += 1;
                    continue;
                };
                if g.is_empty() {
                    empties += 1;
                    continue;
                }
                if has_zm(g) {
                    zm += 1;
                }
                let geometry = convert(g).with_context(|| format!("layer {label}, FID {fid}"))?;
                if attributes {
                    values.push(
                        if let Some(properties) = native
                            .as_ref()
                            .and_then(|n| n.get("properties"))
                            .and_then(|p| p.as_object())
                        {
                            properties
                                .iter()
                                .map(|(name, value)| (name.clone(), browser_json(value.clone())))
                                .collect()
                        } else {
                            feature
                                .fields()
                                .map(|(name, value)| (name, field_json(value)))
                                .collect()
                        },
                    );
                }
                if !has_id {
                    missing.push(features.len());
                }
                features.push((fid, geometry));
            }
            // Allocate fallbacks after reading all real IDs, so negative source IDs and
            // later features cannot collide with the generated identities.
            if !missing.is_empty() {
                let mut used = features.iter().map(|(fid, _)| *fid).collect::<HashSet<_>>();
                let mut candidate = -1_i64;
                for &index in &missing {
                    while used.contains(&candidate) {
                        candidate = candidate
                            .checked_sub(1)
                            .context("no available fallback FID")?;
                    }
                    features[index].0 = candidate;
                    used.insert(candidate);
                }
            }
            if nulls > 0 {
                warnings.push(format!("layer {label}: skipped {nulls} null geometries"));
            }
            if empties > 0 {
                warnings.push(format!("layer {label}: skipped {empties} empty geometries"));
            }
            if zm > 0 {
                warnings.push(format!(
                    "layer {label}: dropped Z/M values from {zm} geometries"
                ));
            }
            if !missing.is_empty() {
                let missing_ids = missing.len();
                warnings.push(format!(
                    "layer {label}: assigned negative FIDs to {missing_ids} features without IDs"
                ));
            }
            if srs.is_none() {
                warnings.push(format!("layer {label}: missing CRS"));
            }
            Ok(ReadLayer {
                layer: Layer::from_features(label, features),
                srs,
                warnings,
                selector: name,
                attributes: values,
            })
        })
        .collect()
}

fn has_zm(g: &gdal::vector::Geometry) -> bool {
    geometry_type_has_z(g.geometry_type())
        || geometry_type_has_m(g.geometry_type())
        || (0..g.geometry_count()).any(|i| has_zm(&g.get_geometry(i)))
}

/// Read XY directly, avoiding the GDAL converter's assumptions about empty rings and Z/M types.
///
/// # Errors
/// Returns an error for unsupported types or malformed collection children.
#[allow(non_upper_case_globals)] // GDAL's generated constants retain their C names.
pub fn convert(g: &gdal::vector::Geometry) -> Result<Geometry> {
    use gdal::vector::geometry_type_flatten;
    use gdal_sys::OGRwkbGeometryType::*;
    let coordinates = || {
        (0..g.point_count())
            .map(|i| {
                let (x, y, _) = g.get_point(i as i32);
                Coord { x, y }
            })
            .collect::<Vec<_>>()
    };
    Ok(match geometry_type_flatten(g.geometry_type()) {
        wkbPoint => {
            let (x, y, _) = g.get_point(0);
            Point::new(x, y).into()
        }
        wkbLineString | wkbLinearRing => LineString(coordinates()).into(),
        wkbPolygon => {
            if g.geometry_count() == 0 {
                Polygon::empty().into()
            } else {
                let mut rings = (0..g.geometry_count()).map(|i| {
                    let ring = g.get_geometry(i);
                    if !matches!(
                        geometry_type_flatten(ring.geometry_type()),
                        wkbLineString | wkbLinearRing
                    ) {
                        bail!("polygon contains a non-linear ring");
                    }
                    let coords = (0..ring.point_count())
                        .map(|j| {
                            let (x, y, _) = ring.get_point(j as i32);
                            Coord { x, y }
                        })
                        .collect();
                    Ok(LineString(coords))
                });
                let exterior = rings.next().expect("nonempty polygon")?;
                Polygon::new(exterior, rings.collect::<Result<Vec<_>>>()?).into()
            }
        }
        wkbMultiPoint => {
            let mut parts = Vec::new();
            for i in 0..g.geometry_count() {
                let child = g.get_geometry(i);
                if child.is_empty() {
                    continue;
                }
                let Geometry::Point(p) = convert(&child)? else {
                    bail!("invalid multipoint part");
                };
                parts.push(p);
            }
            MultiPoint(parts).into()
        }
        wkbMultiLineString => {
            let mut parts = Vec::new();
            for i in 0..g.geometry_count() {
                let Geometry::LineString(p) = convert(&g.get_geometry(i))? else {
                    bail!("invalid multiline part");
                };
                parts.push(p);
            }
            MultiLineString(parts).into()
        }
        wkbMultiPolygon => {
            let mut parts = Vec::new();
            for i in 0..g.geometry_count() {
                let Geometry::Polygon(p) = convert(&g.get_geometry(i))? else {
                    bail!("invalid multipolygon part");
                };
                parts.push(p);
            }
            MultiPolygon(parts).into()
        }
        wkbGeometryCollection => Geometry::GeometryCollection(GeometryCollection(
            (0..g.geometry_count())
                .filter_map(|i| {
                    let child = g.get_geometry(i);
                    (!child.is_empty()).then(|| convert(&child))
                })
                .collect::<Result<_>>()?,
        )),
        kind => bail!(
            "unsupported GDAL geometry type {}",
            gdal::vector::geometry_type_to_name(kind)
        ),
    })
}

/// Return a shared CRS, or no CRS if any input is unreferenced.
///
/// # Errors
/// Returns an error when known input CRSs differ, even if other inputs are unreferenced.
pub fn common_crs(layers: &[ReadLayer]) -> Result<Option<SpatialRef>> {
    let mut first = None;
    let mut missing = false;
    for layer in layers {
        if let Some(srs) = layer.srs.as_ref() {
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
pub fn default_tolerance(srs: Option<&SpatialRef>) -> f64 {
    match srs {
        Some(s) if s.is_geographic() => 1e-8,
        Some(s) if s.is_projected() && s.linear_units().is_finite() && s.linear_units() > 0. => {
            0.001 / s.linear_units()
        }
        _ => 0.,
    }
}

fn field_json(value: Option<gdal::vector::FieldValue>) -> serde_json::Value {
    use gdal::vector::FieldValue::*;
    use serde_json::{Value, json};
    match value {
        None => Value::Null,
        Some(IntegerValue(v)) => json!(v),
        Some(IntegerListValue(v)) => json!(v),
        Some(Integer64Value(v)) => integer_json(v),
        Some(Integer64ListValue(v)) => Value::Array(v.into_iter().map(integer_json).collect()),
        Some(StringValue(v)) => json!(v),
        Some(StringListValue(v)) => json!(v),
        Some(RealValue(v)) => json!(v),
        Some(RealListValue(v)) => json!(v),
        Some(DateValue(v)) => json!(v.to_string()),
        Some(DateTimeValue(v)) => json!(v.to_string()),
    }
}
fn integer_json(v: i64) -> serde_json::Value {
    if v.unsigned_abs() > 9_007_199_254_740_991 {
        serde_json::Value::String(v.to_string())
    } else {
        serde_json::json!(v)
    }
}

fn browser_json(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Number(n)
            if n.as_i64()
                .is_some_and(|v| v.unsigned_abs() > 9_007_199_254_740_991)
                || n.as_u64().is_some_and(|v| v > 9_007_199_254_740_991) =>
        {
            Value::String(n.to_string())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(browser_json).collect()),
        Value::Object(o) => {
            Value::Object(o.into_iter().map(|(k, v)| (k, browser_json(v))).collect())
        }
        v => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layer(epsg: Option<u32>) -> ReadLayer {
        ReadLayer {
            layer: Layer::from_geometries("empty", []),
            srs: epsg.map(|code| SpatialRef::from_epsg(code).unwrap()),
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
                .auth_code()
                .unwrap(),
            3857
        );
        assert!(common_crs(&[]).unwrap().is_none());
    }
}
