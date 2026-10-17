use anyhow::{Context, Result, bail};
use gdal::{
    Dataset, DatasetOptions, GdalOpenFlags,
    vector::{LayerAccess, geometry_type_has_m, geometry_type_has_z},
};
use geo::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};
use std::path::Path;

use crate::read::ReadLayer;

/// Read additional GDAL formats through the common identity and warning pipeline.
pub(crate) fn read_with_attributes(
    path: &Path,
    selector: Option<&str>,
    alias: &str,
    attributes: bool,
) -> Result<Vec<ReadLayer>> {
    crate::gdal_runtime::initialize()?;
    let format = crate::registry::for_path(path).context("unsupported GDAL format")?;
    let allowed = [format.name.as_str()];
    let dataset = Dataset::open_ex(
        path,
        DatasetOptions {
            open_flags: GdalOpenFlags::GDAL_OF_VECTOR,
            allowed_drivers: Some(&allowed),
            ..Default::default()
        },
    )
    .with_context(|| format!("opening {}", path.display()))?;
    let names = if let Some(name) = selector {
        vec![name.to_string()]
    } else {
        dataset.layers().map(|l| l.name()).collect::<Vec<_>>()
    };
    anyhow::ensure!(
        !names.is_empty(),
        "{} contains no vector layers",
        path.display()
    );
    let multiple = names.len() > 1;
    names
        .into_iter()
        .map(|name| {
            let mut layer = dataset
                .layer_by_name(&name)
                .with_context(|| format!("layer {name} in {}", path.display()))?;
            let srs = layer
                .spatial_ref()
                .map(|s| s.to_wkt())
                .transpose()?
                .map(|s| crate::crs::parse(&s))
                .transpose()?;
            let label = if multiple {
                format!("{alias}:{name}")
            } else {
                alias.to_string()
            };
            let mut features = Vec::new();
            for feature in layer.features() {
                // SAFETY: the iterator owns a live feature. GDAL uses -1 as OGRNullFID.
                let fid = unsafe { gdal_sys::OGR_F_GetFID(feature.c_feature()) };
                // SAFETY: avoid wrapping a null OGR geometry handle in gdal 0.19.
                let is_null =
                    unsafe { gdal_sys::OGR_F_GetGeometryRef(feature.c_feature()) }.is_null();
                let geometry = if is_null { None } else { feature.geometry() };
                let zm = geometry.map(|g| has_zm(g, 0)).transpose()?.unwrap_or(false);
                let converted = geometry
                    .map(|g| convert(g, 0))
                    .transpose()
                    .with_context(|| format!("layer {label}, FID {fid}"))?;
                let properties = if attributes {
                    feature
                        .fields()
                        .map(|(name, value)| (name, field_json(value)))
                        .collect()
                } else {
                    serde_json::Map::new()
                };
                features.push(crate::read::RawFeature {
                    id: (fid != -1).then_some(fid),
                    geometry: converted,
                    properties,
                    zm,
                });
            }
            crate::read::finish(
                crate::read::RawLayer {
                    name,
                    srs,
                    features,
                },
                label,
                attributes,
            )
        })
        .collect()
}

fn has_zm(g: &gdal::vector::Geometry, depth: usize) -> Result<bool> {
    anyhow::ensure!(depth <= 64, "GDAL geometry nesting exceeds 64 levels");
    let mut zm = geometry_type_has_z(g.geometry_type()) || geometry_type_has_m(g.geometry_type());
    for i in 0..g.geometry_count() {
        zm |= has_zm(&g.get_geometry(i), depth + 1)?;
    }
    Ok(zm)
}

/// Read XY directly, avoiding the GDAL converter's assumptions about empty rings and Z/M types.
///
/// # Errors
/// Returns an error for unsupported types or malformed collection children.
#[allow(non_upper_case_globals)] // GDAL's generated constants retain their C names.
fn convert(g: &gdal::vector::Geometry, depth: usize) -> Result<Geometry> {
    anyhow::ensure!(depth <= 64, "GDAL geometry nesting exceeds 64 levels");
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
            if g.is_empty() {
                return Ok(Point::new(f64::NAN, f64::NAN).into());
            }
            let (x, y, _) = g.get_point(0);
            Point::new(x, y).into()
        }
        wkbLineString | wkbLinearRing => LineString(coordinates()).into(),
        wkbPolygon => {
            let rings = (0..g.geometry_count())
                .map(|i| {
                    Ok(LineString::try_from(convert(
                        &g.get_geometry(i),
                        depth + 1,
                    )?)?)
                })
                .collect::<Result<Vec<_>>>()?;
            let mut rings = rings.into_iter();
            match rings.next() {
                None => Polygon::empty(),
                Some(exterior) => Polygon::new(exterior, rings.collect()),
            }
            .into()
        }
        wkbMultiPoint => {
            let mut parts = Vec::new();
            for i in 0..g.geometry_count() {
                let child = g.get_geometry(i);
                if child.is_empty() {
                    continue;
                }
                parts.push(Point::try_from(convert(&child, depth + 1)?)?);
            }
            MultiPoint(parts).into()
        }
        wkbMultiLineString => {
            let mut parts = Vec::new();
            for i in 0..g.geometry_count() {
                parts.push(LineString::try_from(convert(
                    &g.get_geometry(i),
                    depth + 1,
                )?)?);
            }
            MultiLineString(parts).into()
        }
        wkbMultiPolygon => {
            let mut parts = Vec::new();
            for i in 0..g.geometry_count() {
                parts.push(Polygon::try_from(convert(&g.get_geometry(i), depth + 1)?)?);
            }
            MultiPolygon(parts).into()
        }
        wkbGeometryCollection => Geometry::GeometryCollection(GeometryCollection(
            (0..g.geometry_count())
                .filter_map(|i| {
                    let child = g.get_geometry(i);
                    (!child.is_empty()).then(|| convert(&child, depth + 1))
                })
                .collect::<Result<_>>()?,
        )),
        kind => bail!(
            "unsupported GDAL geometry type {}",
            gdal::vector::geometry_type_to_name(kind)
        ),
    })
}

fn field_json(value: Option<gdal::vector::FieldValue>) -> serde_json::Value {
    use gdal::vector::FieldValue::*;
    use serde_json::{Value, json};
    match value {
        None => Value::Null,
        Some(IntegerValue(v)) => json!(v),
        Some(IntegerListValue(v)) => json!(v),
        Some(Integer64Value(v)) => json!(v),
        Some(Integer64ListValue(v)) => json!(v),
        Some(StringValue(v)) => json!(v),
        Some(StringListValue(v)) => json!(v),
        Some(RealValue(v)) => json!(v),
        Some(RealListValue(v)) => json!(v),
        Some(DateValue(v)) => json!(v.to_string()),
        Some(DateTimeValue(v)) => json!(v.to_string()),
    }
}
