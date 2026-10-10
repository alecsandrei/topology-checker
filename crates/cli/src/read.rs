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
use std::path::Path;
use topology_checker_core::Layer;

pub struct ReadLayer {
    pub layer: Layer,
    pub srs: Option<SpatialRef>,
    pub warnings: Vec<String>,
}

pub fn read(path: &Path, selector: Option<&str>, alias: &str) -> Result<Vec<ReadLayer>> {
    let dataset = Dataset::open_ex(
        path,
        DatasetOptions {
            open_flags: GdalOpenFlags::GDAL_OF_VECTOR,
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
            let mut nulls = 0;
            let mut empties = 0;
            let mut zm = 0;
            let mut missing_ids = 0;
            for (ordinal, feature) in layer.features().enumerate() {
                let fid = if let Some(fid) = feature.fid() {
                    i64::try_from(fid).context("FID exceeds signed 64-bit range")?
                } else {
                    missing_ids += 1;
                    -(ordinal as i64) - 1
                };
                let Some(g) = feature.geometry() else {
                    nulls += 1;
                    continue;
                };
                if g.is_empty() {
                    empties += 1;
                    continue;
                }
                if geometry_type_has_z(g.geometry_type()) || geometry_type_has_m(g.geometry_type())
                {
                    zm += 1;
                }
                let geometry = convert(g).with_context(|| format!("layer {label}, FID {fid}"))?;
                features.push((fid, geometry));
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
            if missing_ids > 0 {
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
            })
        })
        .collect()
}

/// Read XY directly, avoiding the GDAL converter's assumptions about empty rings and Z/M types.
#[allow(non_upper_case_globals)] // GDAL's generated constants retain their C names.
fn convert(g: &gdal::vector::Geometry) -> Result<Geometry> {
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
                let rings = (0..g.geometry_count())
                    .map(|i| {
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
                    })
                    .collect::<Result<Vec<_>>>()?;
                Polygon::new(rings[0].clone(), rings[1..].to_vec()).into()
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
                .map(|i| convert(&g.get_geometry(i)))
                .collect::<Result<_>>()?,
        )),
        kind => bail!(
            "unsupported GDAL geometry type {}",
            gdal::vector::geometry_type_to_name(kind)
        ),
    })
}

pub fn common_crs(layers: &[ReadLayer]) -> Result<Option<SpatialRef>> {
    let known = layers
        .iter()
        .filter_map(|l| l.srs.as_ref())
        .collect::<Vec<_>>();
    if let Some(first) = known.first() {
        for srs in &known[1..] {
            if *first != *srs {
                bail!("input layers have different CRSs; reprojection is required");
            }
        }
    }
    Ok(if known.len() == layers.len() {
        known.first().map(|s| (*s).clone())
    } else {
        None
    })
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
