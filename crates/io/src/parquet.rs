//! GeoParquet 1.x WKB geometries with Arrow attributes and PROJJSON CRS metadata.
use crate::{
    codec, crs,
    native::stem,
    read::{RawFeature, RawLayer},
    write::{DatasetLayer, FieldType},
};
use anyhow::{Context, Result, bail, ensure};
use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use parquet::{
    arrow::ArrowWriter,
    file::{
        properties::WriterProperties,
        reader::{FileReader, SerializedFileReader},
    },
    format::KeyValue,
    record::Field as ParquetField,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs::File, path::Path, sync::Arc};
pub(crate) fn read(path: &Path) -> Result<Vec<RawLayer>> {
    let reader = SerializedFileReader::new(File::open(path)?)?;
    let metadata = reader
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .context("missing GeoParquet metadata")?;
    let geo: Value = serde_json::from_str(
        metadata
            .iter()
            .find(|m| m.key == "geo")
            .and_then(|m| m.value.as_deref())
            .context("missing GeoParquet geo metadata")?,
    )?;
    ensure!(
        geo["version"].as_str().is_some_and(|s| s.starts_with("1.")),
        "unsupported GeoParquet metadata version"
    );
    let primary = geo["primary_column"]
        .as_str()
        .context("missing GeoParquet primary geometry column")?;
    let column = geo["columns"]
        .get(primary)
        .context("missing GeoParquet geometry column metadata")?;
    ensure!(
        column["encoding"] == "WKB",
        "native GeoParquet supports WKB geometry encoding"
    );
    ensure!(
        column.get("edges").is_none_or(|v| v == "planar"),
        "spherical GeoParquet edges are unsupported"
    );
    let srs = match column.get("crs") {
        None => Some(crs::parse("EPSG:4326")?),
        Some(Value::Null) => None,
        Some(v) => Some(crs::parse(&v.to_string())?),
    };
    let own = metadata
        .iter()
        .find(|m| m.key == "topology_checker")
        .and_then(|m| m.value.as_deref())
        .map(serde_json::from_str::<Value>)
        .transpose()?
        .unwrap_or(Value::Null);
    let identity = own["fid"].as_str();
    let json_fields = own["json_fields"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect::<BTreeSet<_>>())
        .unwrap_or_default();
    let mut features = Vec::new();
    for row in reader.get_row_iter(None)? {
        let row = row?;
        let mut geometry = None;
        let mut zm = false;
        let mut id = None;
        let mut properties = row
            .to_json_value()
            .as_object()
            .context("invalid Parquet row")?
            .clone();
        for (name, field) in row.get_column_iter() {
            if name == primary {
                match field {
                    ParquetField::Null => {}
                    ParquetField::Bytes(b) => {
                        let decoded = codec::decode(b.data())?;
                        geometry = Some(decoded.geometry);
                        zm = decoded.zm;
                    }
                    _ => bail!("GeoParquet WKB column must contain binary values"),
                }
            }
            if identity == Some(name.as_str()) {
                id = match field {
                    ParquetField::Long(n) => Some(*n),
                    ParquetField::Int(n) => Some((*n).into()),
                    ParquetField::Null => None,
                    _ => bail!("GeoParquet identity must be a signed integer"),
                };
            }
        }
        properties.remove(primary);
        if let Some(identity) = identity {
            properties.remove(identity);
        }
        for name in &json_fields {
            if let Some(Value::String(s)) = properties.get(*name) {
                let value = serde_json::from_str(s)?;
                properties.insert((*name).into(), value);
            }
        }
        features.push(RawFeature {
            id,
            geometry,
            properties,
            zm,
        });
    }
    Ok(vec![RawLayer {
        name: own["name"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| stem(path)),
        srs,
        features,
    }])
}
pub(crate) fn write(path: &Path, layer: &DatasetLayer) -> Result<()> {
    write_source(path, layer)
}
pub(crate) fn write_source(path: &Path, source: &dyn crate::write::RecordSource) -> Result<()> {
    let layer = source.metadata();
    let mut geom = "geometry".to_string();
    while layer.fields.iter().any(|f| f.name == geom) {
        geom.push('_');
    }
    let mut fid = "__topology_fid".to_string();
    while layer.fields.iter().any(|f| f.name == fid) || fid == geom {
        fid.push('_');
    }
    let mut fields = vec![
        Field::new(&geom, DataType::Binary, true),
        Field::new(&fid, DataType::Int64, true),
    ];
    for f in &layer.fields {
        fields.push(Field::new(
            &f.name,
            match f.kind {
                FieldType::Integer => DataType::Int64,
                FieldType::Real => DataType::Float64,
                FieldType::Boolean => DataType::Boolean,
                FieldType::Text => DataType::Utf8,
            },
            true,
        ));
    }
    let schema = Arc::new(Schema::new(fields));
    let json_fields = layer
        .fields
        .iter()
        .filter(|f| f.kind == FieldType::Text)
        .map(|f| f.name.clone())
        .collect::<Vec<_>>();
    let mut types = BTreeSet::new();
    for row in source.records() {
        let row = row?;
        if let Some(g) = &row.geometry {
            types.insert(match codec::type_name(g) {
                "POINT" => "Point",
                "LINESTRING" => "LineString",
                "POLYGON" => "Polygon",
                "MULTIPOINT" => "MultiPoint",
                "MULTILINESTRING" => "MultiLineString",
                "MULTIPOLYGON" => "MultiPolygon",
                _ => "GeometryCollection",
            });
        }
    }
    let crs = layer
        .srs
        .as_ref()
        .map(|s| {
            s.to_projjson()
                .and_then(|s| Ok(serde_json::from_str::<Value>(&s)?))
        })
        .transpose()?;
    let geo = json!({"version":"1.1.0","primary_column":geom,"columns":{geom.clone():{"encoding":"WKB","geometry_types":types,"crs":crs,"edges":"planar"}}});
    let own = json!({"fid":fid,"name":layer.name,"json_fields":json_fields});
    let props = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![
            KeyValue::new("geo".into(), geo.to_string()),
            KeyValue::new("topology_checker".into(), own.to_string()),
        ]))
        .build();
    let mut writer = ArrowWriter::try_new(File::create(path)?, Arc::clone(&schema), Some(props))?;
    let mut batch = Vec::with_capacity(1024);
    let mut offset = 0;
    for row in source.records() {
        batch.push(row?);
        if batch.len() == 1024 {
            writer.write(&record_batch(layer, &schema, &batch, offset)?)?;
            offset += batch.len();
            batch.clear();
        }
    }
    if !batch.is_empty() || offset == 0 {
        writer.write(&record_batch(layer, &schema, &batch, offset)?)?;
    }
    writer.close()?;
    Ok(())
}

fn record_batch(
    layer: &DatasetLayer,
    schema: &Arc<Schema>,
    records: &[crate::write::Record],
    offset: usize,
) -> Result<RecordBatch> {
    let mut arrays: Vec<ArrayRef> = Vec::new();
    let wkb = records
        .iter()
        .map(|r| r.geometry.as_ref().map(codec::encode).transpose())
        .collect::<Result<Vec<_>>>()?;
    arrays.push(Arc::new(BinaryArray::from_iter(
        wkb.iter().map(|b| b.as_deref()),
    )));
    arrays.push(Arc::new(Int64Array::from_iter(
        records
            .iter()
            .enumerate()
            .map(|(i, r)| r.id.or(Some((offset + i) as i64))),
    )));
    for f in &layer.fields {
        let values = records.iter().map(|r| r.properties.get(&f.name));
        let (ty, array): (DataType, ArrayRef) = match f.kind {
            FieldType::Integer => (
                DataType::Int64,
                Arc::new(Int64Array::from_iter(
                    values.map(|v| v.and_then(Value::as_i64)),
                )),
            ),
            FieldType::Real => (
                DataType::Float64,
                Arc::new(Float64Array::from_iter(
                    values.map(|v| v.and_then(Value::as_f64)),
                )),
            ),
            FieldType::Boolean => (
                DataType::Boolean,
                Arc::new(BooleanArray::from_iter(
                    values.map(|v| v.and_then(Value::as_bool)),
                )),
            ),
            FieldType::Text => (
                DataType::Utf8,
                Arc::new(StringArray::from_iter(
                    values.map(|v| v.filter(|v| !v.is_null()).map(Value::to_string)),
                )),
            ),
        };
        let _ = ty;
        arrays.push(array);
    }
    Ok(RecordBatch::try_new(Arc::clone(schema), arrays)?)
}
