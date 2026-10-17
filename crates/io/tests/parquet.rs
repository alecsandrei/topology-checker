use arrow_array::{ArrayRef, BinaryArray, Int32Array, RecordBatch, StringArray};
use arrow_schema::{Field, Schema};
use parquet::{arrow::ArrowWriter, file::properties::WriterProperties, format::KeyValue};
use std::{path::Path, sync::Arc};
use topology_checker_core::geo::{Geometry, Point};
use topology_checker_io::{
    crs, read,
    write::{self, DatasetLayer, Record},
};

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut b = vec![1, 1, 0, 0, 0];
    b.extend(x.to_le_bytes());
    b.extend(y.to_le_bytes());
    b
}
/// Write a GeoParquet file by hand, as another producer would.
fn foreign(path: &Path, geo: &str, own: Option<&str>, geometry: ArrayRef, id: ArrayRef) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("geometry", geometry.data_type().clone(), true),
        Field::new("id", id.data_type().clone(), true),
    ]));
    let mut metadata = vec![KeyValue::new("geo".into(), geo.to_string())];
    metadata.extend(own.map(|own| KeyValue::new("topology_checker".into(), own.to_string())));
    let props = WriterProperties::builder()
        .set_key_value_metadata(Some(metadata))
        .build();
    let mut writer = ArrowWriter::try_new(
        std::fs::File::create(path).unwrap(),
        Arc::clone(&schema),
        Some(props),
    )
    .unwrap();
    writer
        .write(&RecordBatch::try_new(schema, vec![geometry, id]).unwrap())
        .unwrap();
    writer.close().unwrap();
}
fn geo(crs: &str) -> String {
    format!(
        r#"{{"version":"1.1.0","primary_column":"geometry","columns":{{"geometry":{{"encoding":"WKB","geometry_types":[]{crs}}}}}}}"#
    )
}
fn wkb(values: &[Option<Vec<u8>>]) -> ArrayRef {
    Arc::new(BinaryArray::from_iter(values.iter().map(|v| v.as_deref())))
}

#[test]
fn foreign_files_default_to_wgs84_and_accept_null_geometries_and_integer_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("foreign.parquet");
    foreign(
        &path,
        &geo(""),
        Some(r#"{"fid":"id"}"#),
        wkb(&[Some(point_wkb(5., 52.)), None, Some(point_wkb(6., 53.))]),
        Arc::new(Int32Array::from(vec![Some(7), Some(8), None])),
    );
    let layers = read::read(&path, None, "x").unwrap();
    assert_eq!(layers[0].srs.as_ref().unwrap().epsg(), Some(4326));
    assert!(layers[0].warnings.iter().any(|w| w.contains("null")));
    let ids = layers[0]
        .layer
        .features()
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&7));

    let path = dir.path().join("no-crs.parquet");
    foreign(
        &path,
        &geo(r#","crs":null"#),
        None,
        wkb(&[Some(point_wkb(5., 52.))]),
        Arc::new(Int32Array::from(vec![1])),
    );
    assert_eq!(read::read(&path, None, "x").unwrap()[0].srs, None);
}

#[test]
fn unsupported_geometry_and_identity_columns_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text-geometry.parquet");
    foreign(
        &path,
        &geo(""),
        None,
        Arc::new(StringArray::from(vec!["POINT (1 2)"])),
        Arc::new(Int32Array::from(vec![1])),
    );
    let error = format!("{:#}", read::read(&path, None, "x").unwrap_err());
    assert!(error.contains("binary"), "{error}");

    let path = dir.path().join("text-id.parquet");
    foreign(
        &path,
        &geo(""),
        Some(r#"{"fid":"id"}"#),
        wkb(&[Some(point_wkb(1., 2.))]),
        Arc::new(StringArray::from(vec!["a"])),
    );
    let error = format!("{:#}", read::read(&path, None, "x").unwrap_err());
    assert!(error.contains("signed integer"), "{error}");

    let path = dir.path().join("bad-metadata.parquet");
    foreign(
        &path,
        "not json",
        None,
        wkb(&[Some(point_wkb(1., 2.))]),
        Arc::new(Int32Array::from(vec![1])),
    );
    assert!(read::read(&path, None, "x").is_err());
}

#[test]
fn reserved_column_names_and_large_layers_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let records = (0..1500)
        .map(|i| Record {
            id: Some(i + 1),
            geometry: Some(Geometry::from(Point::new(i as f64, 52.))),
            properties: serde_json::json!({"geometry": "mine", "__topology_fid": i})
                .as_object()
                .unwrap()
                .clone(),
        })
        .collect();
    let layer = DatasetLayer::new("big", Some(crs::parse("EPSG:4326").unwrap()), records);
    let path = dir.path().join("big.parquet");
    write::dataset(&path, None, false, &[layer]).unwrap();
    let read = read::read_with_attributes(&path, None, "x", true).unwrap();
    assert_eq!(read[0].layer.len(), 1500);
    assert_eq!(read[0].attributes[0]["geometry"], "mine");
    assert_eq!(read[0].attributes[1499]["__topology_fid"], 1499);
}
