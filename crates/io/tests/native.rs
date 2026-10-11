use geo::{
    Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point,
    Polygon, polygon,
};
use serde_json::{Map, Value, json};
use topology_checker_io::{
    crs, read,
    write::{self, DatasetLayer, Record},
};
fn geometries() -> Vec<Geometry> {
    let line = LineString::from(vec![(5., 52.), (6., 53.)]);
    let polygon = polygon![(x:5.,y:52.),(x:6.,y:52.),(x:6.,y:53.),(x:5.,y:52.)];
    vec![
        Point::new(5., 52.).into(),
        line.clone().into(),
        polygon.clone().into(),
        MultiPoint(vec![Point::new(5., 52.), Point::new(6., 53.)]).into(),
        MultiLineString(vec![line.clone()]).into(),
        MultiPolygon(vec![polygon]).into(),
        Geometry::GeometryCollection(GeometryCollection(vec![
            Point::new(5., 52.).into(),
            line.into(),
        ])),
    ]
}
fn dataset(name: &str, geometries: Vec<Geometry>) -> DatasetLayer {
    DatasetLayer::new(name,Some(crs::parse("EPSG:4326").unwrap()),geometries.into_iter().enumerate().map(|(i,g)|Record{id:Some(-(i as i64)-1),geometry:Some(g),properties:json!({"label":"café","number":i,"real":1.5,"flag":true,"nullable":null,"large":9007199254740993i64,"nested":{"a":[1,null,"yes"]}}).as_object().unwrap().clone()}).collect())
}
#[test]
fn basic_types_collections_attributes_and_signed_ids_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["geojson", "gpkg", "fgb", "parquet", "kml", "kmz"] {
        let path = dir.path().join(format!("all.{ext}"));
        let original = dataset("all", geometries());
        write::dataset(&path, None, false, std::slice::from_ref(&original))
            .unwrap_or_else(|e| panic!("{ext}: {e:#}"));
        let retained = read::read_with_attributes(&path, None, "all", true)
            .unwrap_or_else(|e| panic!("{ext}: {e:#}"));
        let retained = &retained[0];
        assert_eq!(retained.layer.len(), original.records.len(), "{ext}");
        for (row, (id, g)) in retained.layer.features().enumerate() {
            let i = original
                .records
                .iter()
                .position(|r| r.id == Some(id))
                .unwrap();
            assert_eq!(id, original.records[i].id.unwrap(), "{ext}");
            assert_eq!(
                g,
                original.records[i].geometry.as_ref().unwrap(),
                "{ext} geometry {i}"
            );
            for key in ["label", "number", "real", "flag", "nullable", "large"] {
                let expected = if key == "large" {
                    Value::String("9007199254740993".into())
                } else {
                    original.records[i].properties[key].clone()
                };
                assert_eq!(
                    retained.attributes[row][key], expected,
                    "{ext} attribute {key}"
                );
            }
        }
        assert_eq!(retained.srs, original.srs, "{ext}");
        let without = read::read(&path, None, "all").unwrap();
        assert_eq!(
            without[0]
                .layer
                .features()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            retained
                .layer
                .features()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        );
    }
}
#[test]
fn projected_crs_empty_datasets_and_selection() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["geojson", "gpkg", "fgb", "parquet"] {
        let path = dir.path().join(format!("empty.{ext}"));
        let mut layer = dataset("empty", vec![]);
        layer.srs = Some(crs::parse("EPSG:28992").unwrap());
        write::dataset(&path, None, false, &[layer.clone()]).unwrap();
        let layers = read::read(&path, None, "empty").unwrap();
        assert_eq!(layers[0].srs, layer.srs, "{ext}");
        assert!(layers[0].layer.is_empty());
        assert!(read::read(&path, Some("missing"), "empty").is_err());
    }
    let path = dir.path().join("multi.gpkg");
    let layers = vec![
        dataset("first", vec![Point::new(5., 52.).into()]),
        dataset("second", vec![]),
    ];
    write::dataset(&path, None, false, &layers).unwrap();
    let read = read::read(&path, None, "input").unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].layer.name(), "input:first");
    assert_eq!(
        read::read(&path, Some("second"), "selected").unwrap()[0]
            .layer
            .name(),
        "selected"
    );
}
#[test]
fn geojson_identity_is_independent_of_inspection_and_fallbacks_do_not_collide() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ids.geojson");
    std::fs::write(&path,r#"{"type":"FeatureCollection","features":[{"type":"Feature","id":-1,"properties":{},"geometry":{"type":"Point","coordinates":[1,1]}},{"type":"Feature","id":"nonnumeric","properties":{},"geometry":{"type":"Point","coordinates":[2,2]}},{"type":"Feature","id":-2,"properties":{},"geometry":{"type":"Point","coordinates":[3,3]}},{"type":"Feature","id":"9007199254740993","properties":{},"geometry":{"type":"Point","coordinates":[4,4]}}]}"#).unwrap();
    for attributes in [false, true] {
        let l = read::read_with_attributes(&path, None, "ids", attributes).unwrap();
        assert_eq!(
            l[0].layer.features().map(|(id, _)| id).collect::<Vec<_>>(),
            vec![-1, -3, -2, 9007199254740993]
        );
    }
    let duplicate = r#"{"type":"FeatureCollection","features":[{"type":"Feature","id":-1,"properties":{},"geometry":null},{"type":"Feature","id":-1,"properties":{},"geometry":{"type":"Point","coordinates":[1,1]}}]}"#;
    std::fs::write(&path, duplicate).unwrap();
    assert!(
        read::read(&path, None, "ids")
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}
#[test]
fn null_and_empty_geometry_warnings_keep_attributes_aligned() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["gpkg", "geojson", "parquet", "fgb"] {
        let path = dir.path().join(format!("nulls.{ext}"));
        let mut l = dataset(
            "nulls",
            vec![
                Point::new(5., 52.).into(),
                Polygon::empty().into(),
                Point::new(6., 53.).into(),
            ],
        );
        l.records.insert(
            0,
            Record {
                id: Some(-100),
                geometry: None,
                properties: Map::new(),
            },
        );
        write::dataset(&path, None, false, &[l]).unwrap();
        let read = read::read_with_attributes(&path, None, "nulls", true).unwrap();
        assert_eq!(read[0].layer.len(), 2, "{ext}");
        assert_eq!(read[0].attributes.len(), 2, "{ext}");
        assert!(
            read[0]
                .warnings
                .iter()
                .any(|w| w.contains("null geometries")),
            "{ext}"
        );
        assert!(
            read[0]
                .warnings
                .iter()
                .any(|w| w.contains("empty geometries")),
            "{ext}"
        );
    }
}
#[test]
fn geopackage_large_records_readonly_and_standalone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.gpkg");
    let mut l = dataset("quote\"layer", vec![Point::new(5., 52.).into()]);
    l.records[0]
        .properties
        .insert("long".into(), "x".repeat(2_000_000).into());
    l = DatasetLayer::new(&l.name, l.srs, l.records);
    write::dataset(&path, None, false, &[l]).unwrap();
    assert!(!path.with_extension("gpkg-wal").exists());
    let before = std::fs::read(&path).unwrap();
    let copied = dir.path().join("standalone.gpkg");
    std::fs::write(&copied, &before).unwrap();
    let read = read::read_with_attributes(&copied, None, "big", true).unwrap();
    assert_eq!(
        read[0].attributes[0]["long"].as_str().unwrap().len(),
        2_000_000
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read(&copied).unwrap(), before);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "read-only imports must not create sidecars"
    );
}
#[test]
fn failed_writes_preserve_destination_and_overwrite_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.gpkg");
    let original = dataset("original", vec![Point::new(1., 2.).into()]);
    write::dataset(&path, None, false, std::slice::from_ref(&original)).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(write::dataset(&path, None, false, std::slice::from_ref(&original)).is_err());
    let mut bad = original;
    bad.fields[0].name = "fid".into();
    assert!(write::dataset(&path, None, true, &[bad]).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    write::dataset(&path, None, true, &[dataset("replacement", vec![])]).unwrap();
    assert_eq!(
        read::read(&path, None, "read").unwrap()[0].selector,
        "replacement"
    );
}
#[test]
fn gpx_reprojects_lines_and_rejects_polygons() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lines.gpx");
    let mut l = dataset(
        "lines",
        vec![
            Point::new(155000., 463000.).into(),
            LineString::from(vec![(155000., 463000.), (156000., 464000.)]).into(),
        ],
    );
    l.srs = Some(crs::parse("EPSG:28992").unwrap());
    write::dataset(&path, None, false, &[l]).unwrap();
    let read = read::read_with_attributes(&path, None, "gpx", true).unwrap();
    assert_eq!(read[0].layer.len(), 2);
    let Geometry::Point(p) = read[0].layer.features().next().unwrap().1 else {
        panic!()
    };
    assert!((p.x() - 5.3872).abs() < 0.001);
    assert!((p.y() - 52.1552).abs() < 0.001);
    let before = std::fs::read(&path).unwrap();
    assert!(
        write::dataset(
            &path,
            None,
            true,
            &[dataset("polygon", vec![geometries()[2].clone()])]
        )
        .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}
#[test]
fn malformed_native_input_never_switches_backends() {
    let dir = tempfile::tempdir().unwrap();
    for ext in [
        "geojson", "gpkg", "fgb", "parquet", "kml", "kmz", "gpx", "shp",
    ] {
        let path = dir.path().join(format!("bad.{ext}"));
        std::fs::write(&path, b"<OGRVRTDataSource></OGRVRTDataSource>").unwrap();
        assert!(read::read(&path, None, "bad").is_err(), "{ext}");
    }
}
#[test]
fn crs_bounds_metadata_and_rd_roundtrip() {
    let from = crs::parse("EPSG:28992").unwrap();
    let wgs = crs::parse("EPSG:4326").unwrap();
    assert_eq!(crs::parse(&from.to_wkt().unwrap()).unwrap(), from);
    let t = crs::Transform::new(&from, &wgs).unwrap();
    let p = crs::transform(&Point::new(155000., 463000.).into(), &t).unwrap();
    let back = crs::transform(&p, &crs::Transform::new(&wgs, &from).unwrap()).unwrap();
    let Geometry::Point(back) = back else {
        panic!()
    };
    assert!((back.x() - 155000.).abs() < 0.01);
    assert!((back.y() - 463000.).abs() < 0.01);
    let b = t
        .transform_bounds(&[154000., 462000., 156000., 464000.], 21)
        .unwrap();
    assert!(b[0] < 5.387 && b[2] > 5.388 && b[1] < 52.155 && b[3] > 52.155);
    assert!(t.transform_bounds(&[2., 2., 1., 1.], 21).is_err());
    assert!(crs::parse("EPSG:0").is_err());
}
#[test]
fn nested_collections_keep_structure_and_truncated_inputs_return_errors() {
    let dir = tempfile::tempdir().unwrap();
    let nested = Geometry::GeometryCollection(GeometryCollection(vec![
        Point::new(5., 52.).into(),
        Geometry::GeometryCollection(GeometryCollection(vec![
            geometries()[1].clone(),
            geometries()[2].clone(),
        ])),
    ]));
    for ext in ["geojson", "gpkg", "fgb", "parquet", "kml", "kmz"] {
        let path = dir.path().join(format!("nested.{ext}"));
        write::dataset(
            &path,
            None,
            false,
            &[dataset("nested", vec![nested.clone()])],
        )
        .unwrap();
        let read = read::read(&path, None, "nested").unwrap();
        assert_eq!(read[0].layer.features().next().unwrap().1, &nested, "{ext}");
        let bytes = std::fs::read(&path).unwrap();
        for length in [0, 8, bytes.len() / 2] {
            let bad = dir.path().join(format!("bad.{ext}"));
            std::fs::write(&bad, &bytes[..length.min(bytes.len())]).unwrap();
            assert!(read::read(&bad, None, "bad").is_err(), "{ext} at {length}");
        }
    }
}
#[test]
fn generated_output_ids_reserve_explicit_negative_ids() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["geojson", "gpkg", "fgb", "parquet", "kml", "kmz", "gpx"] {
        let path = dir.path().join(format!("ids.{ext}"));
        let mut l = dataset("ids", vec![Point::new(5., 52.).into(); 4]);
        l.records[0].id = None;
        l.records[1].id = Some(-1);
        l.records[2].id = None;
        l.records[3].id = Some(-2);
        write::dataset(&path, None, false, &[l]).unwrap();
        let read = read::read(&path, None, "ids").unwrap();
        let ids = read[0]
            .layer
            .features()
            .map(|(id, _)| id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids, [-1, -2, -3, -4].into_iter().collect(), "{ext}");
    }
}
#[test]
fn shapefile_keeps_exact_integer_attributes_and_rejects_truncated_dbf() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("integers.shp");
    let values = [i64::MIN, -9007199254740993, 9007199254740993, i64::MAX];
    let l = DatasetLayer::new(
        "integers",
        Some(crs::parse("EPSG:4326").unwrap()),
        values
            .iter()
            .map(|n| Record {
                id: None,
                geometry: Some(Point::new(5., 52.).into()),
                properties: json!({"integer": n}).as_object().unwrap().clone(),
            })
            .collect(),
    );
    write::dataset(&path, None, false, &[l]).unwrap();
    let read = read::read_with_attributes(&path, None, "integers", true).unwrap();
    for (i, value) in values.iter().enumerate() {
        assert_eq!(read[0].attributes[i]["integer"], value.to_string());
    }
    let dbf = path.with_extension("dbf");
    let bytes = std::fs::read(&dbf).unwrap();
    std::fs::write(dbf, &bytes[..bytes.len() / 2]).unwrap();
    assert!(read::read(&path, None, "truncated").is_err());
}
