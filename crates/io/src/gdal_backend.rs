//! Optional additional drivers. Registry dispatch never sends native formats here.
use crate::{
    registry::Format,
    write::{DatasetLayer, FieldType},
};
use anyhow::{Context, Result};
use gdal::{
    DriverManager, Metadata,
    vector::{Feature, LayerAccess, LayerOptions, OGRFieldType, OGRwkbGeometryType, ToGdal},
};
use std::path::Path;
pub(crate) fn formats() -> Vec<Format> {
    (0..DriverManager::count())
        .filter_map(|i| DriverManager::get_driver(i).ok())
        .filter(|d| d.metadata_item("DCAP_VECTOR", "").as_deref() == Some("YES"))
        .map(|d| Format {
            name: d.short_name(),
            extensions: d
                .metadata_item("DMD_EXTENSIONS", "")
                .or_else(|| d.metadata_item("DMD_EXTENSION", ""))
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_ascii_lowercase)
                .collect(),
            read: d.metadata_item("DCAP_OPEN", "").as_deref() == Some("YES"),
            write: d.metadata_item("DCAP_CREATE", "").as_deref() == Some("YES"),
            layers: d
                .metadata_item("DCAP_MULTIPLE_VECTOR_LAYERS", "")
                .as_deref()
                == Some("YES"),
            collections: true,
            polygons: true,
            web_export: false,
            native: false,
        })
        .collect()
}
pub(crate) fn write(path: &Path, driver: &str, layers: &[DatasetLayer]) -> Result<()> {
    let sources = layers
        .iter()
        .map(|l| l as &dyn crate::write::RecordSource)
        .collect::<Vec<_>>();
    write_sources(path, driver, &sources)
}
pub(crate) fn write_sources(
    path: &Path,
    driver: &str,
    layers: &[&dyn crate::write::RecordSource],
) -> Result<()> {
    crate::gdal_runtime::initialize()?;
    let mut ds = DriverManager::get_driver_by_name(driver)?.create_vector_only(path)?;
    for source in layers {
        let l = source.metadata();
        let srs = l.srs.as_ref().map(|s| s.to_gdal()).transpose()?;
        let mut options = Vec::new();
        if driver == "CSV" {
            options.push("GEOMETRY=AS_WKT");
        }
        if driver == "OpenFileGDB"
            && gdal::version::version_info("VERSION_NUM")
                .parse::<u32>()
                .unwrap_or(0)
                >= 3_090_000
        {
            options.push("TARGET_ARCGIS_VERSION=ARCGIS_PRO_3_2_OR_LATER");
        }
        let mut inferred = None;
        let mut integer64 = vec![false; l.fields.len()];
        for row in source.records() {
            let row = row?;
            // Infer ranges from repeatable records, including disk-backed sources.
            for (i, field) in l.fields.iter().enumerate() {
                if field.kind == FieldType::Integer
                    && !integer64[i]
                    && row
                        .properties
                        .get(&field.name)
                        .and_then(serde_json::Value::as_i64)
                        .is_some_and(|n| i32::try_from(n).is_err())
                {
                    integer64[i] = true;
                }
            }
            if let Some(g) = &row.geometry {
                let ty = g.to_gdal()?.geometry_type();
                inferred = Some(inferred.map_or(ty, |old| {
                    if old == ty {
                        ty
                    } else {
                        OGRwkbGeometryType::wkbUnknown
                    }
                }));
            }
        }
        let inferred = inferred.unwrap_or(OGRwkbGeometryType::wkbUnknown);
        let ty = match l.geometry_type.as_deref() {
            Some("POINT") => OGRwkbGeometryType::wkbPoint,
            Some("LINESTRING") => OGRwkbGeometryType::wkbLineString,
            Some("POLYGON") => OGRwkbGeometryType::wkbPolygon,
            Some("MULTIPOINT") => OGRwkbGeometryType::wkbMultiPoint,
            Some("MULTILINESTRING") => OGRwkbGeometryType::wkbMultiLineString,
            Some("MULTIPOLYGON") => OGRwkbGeometryType::wkbMultiPolygon,
            Some("GEOMETRYCOLLECTION") => OGRwkbGeometryType::wkbGeometryCollection,
            _ => inferred,
        };
        let layer = ds.create_layer(LayerOptions {
            name: &l.name,
            srs: srs.as_ref(),
            ty,
            options: (!options.is_empty()).then_some(options.as_slice()),
        })?;
        layer.create_defn_fields(
            &l.fields
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    (
                        f.name.as_str(),
                        match f.kind {
                            FieldType::Boolean => OGRFieldType::OFTInteger,
                            FieldType::Integer if !integer64[i] => OGRFieldType::OFTInteger,
                            FieldType::Integer => OGRFieldType::OFTInteger64,
                            FieldType::Real => OGRFieldType::OFTReal,
                            FieldType::Text => OGRFieldType::OFTString,
                        },
                    )
                })
                .collect::<Vec<_>>(),
        )?;
        for r in source.records() {
            let r = r?;
            let mut f = Feature::new(layer.defn())?;
            if let Some(g) = &r.geometry {
                f.set_geometry(g.to_gdal()?)?;
            }
            if let Some(id) = r.id {
                // SAFETY: the feature owns a live OGR handle, and OGR accepts a signed FID.
                let error = unsafe { gdal_sys::OGR_F_SetFID(f.c_feature(), id) };
                anyhow::ensure!(error == 0, "GDAL driver rejected feature ID {id}");
            }
            for (i, field) in l.fields.iter().enumerate() {
                let v = r
                    .properties
                    .get(&field.name)
                    .unwrap_or(&serde_json::Value::Null);
                match field.kind {
                    _ if v.is_null() => f.set_field_null(i)?,
                    FieldType::Integer => {
                        f.set_field_integer64(i, v.as_i64().context("integer attribute required")?)?
                    }
                    FieldType::Boolean => f.set_field_integer64(
                        i,
                        i64::from(v.as_bool().context("boolean attribute required")?),
                    )?,
                    FieldType::Real => {
                        f.set_field_double(i, v.as_f64().context("numeric attribute required")?)?
                    }
                    FieldType::Text => f.set_field_string(
                        i,
                        &v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string()),
                    )?,
                }
            }
            f.create(&layer)?;
        }
    }
    ds.flush_cache()?;
    Ok(())
}
