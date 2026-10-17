#![cfg(feature = "gdal-system")]
//! Interoperability exercises real GDAL datasets while dispatch remains native.
use gdal::{
    Dataset, DriverManager,
    spatial_ref::{AxisMappingStrategy, SpatialRef},
    vector::{
        Feature, LayerAccess, LayerOptions, OGRFieldType, OGRwkbGeometryType, ToGdal, sql::Dialect,
    },
};
use geo::{
    Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point,
    polygon,
};
use serde_json::json;
use std::path::Path;
use topology_checker_core::{Layer, Rule, Topology};
use topology_checker_io::{
    crs, read, registry,
    report_store::DiskReport,
    write::{self, DatasetLayer, Field, FieldType, Record, ReportLayout},
};
fn sample_geometries() -> Vec<Geometry> {
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
fn layer(geometries: Vec<Geometry>) -> DatasetLayer {
    DatasetLayer::new("features",Some(crs::parse("EPSG:4326").unwrap()),geometries.into_iter().enumerate().map(|(i,g)|Record{id:Some(i as i64+10),geometry:Some(g),properties:json!({"label":"café & <GIS>","integer":9007199254740993i64,"real":1.25,"nullable":null,"boolean":true}).as_object().unwrap().clone()}).collect())
}
fn gdal_geometry(g: &gdal::vector::Geometry) -> Geometry {
    // GDAL's standard conversion can handle all nonempty XY geometries here.
    g.to_geo().unwrap()
}
fn sqlite_check(path: &Path) {
    let output=std::process::Command::new("sqlite3").arg(path).arg("PRAGMA integrity_check; PRAGMA foreign_key_check; PRAGMA application_id; PRAGMA user_version;").output().expect("the GDAL interoperability job requires sqlite3");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "ok\n1196444487\n10400\n"
    );
}
#[test]
fn every_basic_geometry_written_natively_is_readable_by_gdal() {
    let dir = tempfile::tempdir().unwrap();
    for (driver, extension) in [
        ("GeoJSON", "geojson"),
        ("GPKG", "gpkg"),
        ("FlatGeobuf", "fgb"),
    ] {
        for (i, g) in sample_geometries().into_iter().enumerate() {
            let path = dir.path().join(format!("{i}.{extension}"));
            let original = layer(vec![g.clone()]);
            write::dataset(&path, None, false, &[original]).unwrap();
            let ds = Dataset::open(&path).unwrap();
            assert_eq!(ds.driver().short_name(), driver);
            let mut l = ds.layer(0).unwrap();
            assert_eq!(l.feature_count(), 1);
            let feature = l.features().next().unwrap();
            assert_eq!(
                gdal_geometry(feature.geometry().unwrap()),
                g,
                "{driver} geometry {i}"
            );
            assert_eq!(
                feature
                    .field_as_string(feature.field_index("label").unwrap())
                    .unwrap()
                    .unwrap(),
                "café & <GIS>"
            );
            assert_eq!(
                feature
                    .field_as_integer64(feature.field_index("integer").unwrap())
                    .unwrap()
                    .unwrap(),
                9007199254740993,
                "{driver} exact integer"
            );
            if extension == "gpkg" {
                sqlite_check(&path);
            }
        }
    }
}
#[test]
fn indexed_gdal_geopackages_preserve_signed_ids_multilayers_blobs_and_readonly_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("indexed.gpkg");
    {
        let mut ds = DriverManager::get_driver_by_name("GPKG")
            .unwrap()
            .create_vector_only(&path)
            .unwrap();
        let srs = SpatialRef::from_epsg(28992).unwrap();
        for (index, name) in ["signed", "large", "empty"].into_iter().enumerate() {
            let l = ds
                .create_layer(LayerOptions {
                    name,
                    srs: Some(&srs),
                    ty: OGRwkbGeometryType::wkbUnknown,
                    ..Default::default()
                })
                .unwrap();
            l.create_defn_fields(&[
                ("label", OGRFieldType::OFTString),
                ("blob", OGRFieldType::OFTBinary),
                ("large_id", OGRFieldType::OFTInteger64),
            ])
            .unwrap();
            if index == 2 {
                continue;
            }
            for (id, wkt) in [
                (-2, "POINT (155000 463000)"),
                (
                    -12,
                    "GEOMETRYCOLLECTION (POINT (155001 463001), LINESTRING (155000 463000, 155001 463001))",
                ),
                (9007199254740993, "POINT (155002 463002)"),
            ] {
                let mut f = Feature::new(l.defn()).unwrap();
                f.set_geometry(gdal::vector::Geometry::from_wkt(wkt).unwrap())
                    .unwrap();
                let label = if index == 1 {
                    "x".repeat(1_100_000)
                } else {
                    "retained".into()
                };
                f.set_field_string(0, &label).unwrap();
                f.set_field_integer64(2, id).unwrap();
                // SAFETY: f owns a live feature; the buffers outlive each setter call.
                unsafe {
                    assert_eq!(gdal_sys::OGR_F_SetFID(f.c_feature(), id), 0);
                    let bytes = vec![0x42u8; 70_000];
                    gdal_sys::OGR_F_SetFieldBinary(
                        f.c_feature(),
                        1,
                        bytes.len() as i32,
                        bytes.as_ptr().cast_mut().cast(),
                    );
                }
                f.create(&l).unwrap();
            }
        }
        ds.flush_cache().unwrap();
        ds.execute_sql(
            "UPDATE signed SET fid = -1 WHERE fid = -2",
            None,
            Dialect::SQLITE,
        )
        .unwrap();
        ds.execute_sql(
            "UPDATE large SET fid = -1 WHERE fid = -2",
            None,
            Dialect::SQLITE,
        )
        .unwrap();
        // Verify the fixture really has an R-tree, so schema compatibility is exercised.
        let mut result = ds
            .execute_sql(
                "SELECT name FROM sqlite_master WHERE name LIKE 'rtree_signed_geom%'",
                None,
                Dialect::SQLITE,
            )
            .unwrap()
            .unwrap();
        assert!(result.features().count() >= 4);
    }
    let before = std::fs::read(&path).unwrap();
    let imported = read::read_with_attributes(&path, None, "fixture", true).unwrap();
    assert_eq!(imported.len(), 3);
    let signed = imported.iter().find(|l| l.selector == "signed").unwrap();
    assert_eq!(
        signed
            .layer
            .features()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        vec![-12, -1, 9007199254740993]
    );
    assert_eq!(signed.attributes[0]["large_id"], -12);
    assert_eq!(
        signed.attributes[0]["blob"].as_array().unwrap().len(),
        70_000
    );
    assert_eq!(signed.srs, Some(crs::parse("EPSG:28992").unwrap()));
    assert_eq!(
        read::read(&path, Some("large"), "selected").unwrap().len(),
        1
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "read-only native imports create no sidecars"
    );
}
#[test]
fn gdal_created_z_m_null_and_empty_geopackage_input_has_aligned_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dimensions.gpkg");
    {
        let mut ds = DriverManager::get_driver_by_name("GPKG")
            .unwrap()
            .create_vector_only(&path)
            .unwrap();
        let l = ds
            .create_layer(LayerOptions {
                name: "dimensions",
                ty: OGRwkbGeometryType::wkbUnknown,
                ..Default::default()
            })
            .unwrap();
        l.create_defn_fields(&[("name", OGRFieldType::OFTString)])
            .unwrap();
        for (i, wkt) in [
            Some("POINT Z (1 2 3)"),
            Some("LINESTRING M (1 2 3, 4 5 6)"),
            None,
            Some("POINT EMPTY"),
            Some("POLYGON EMPTY"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut f = Feature::new(l.defn()).unwrap();
            f.set_field_string(0, &i.to_string()).unwrap();
            if let Some(wkt) = wkt {
                f.set_geometry(gdal::vector::Geometry::from_wkt(wkt).unwrap())
                    .unwrap();
            }
            f.create(&l).unwrap();
        }
    }
    let l = read::read_with_attributes(&path, None, "dimensions", true)
        .unwrap()
        .remove(0);
    assert_eq!(l.layer.len(), 2);
    assert_eq!(l.attributes.len(), 2);
    assert_eq!(l.attributes[1]["name"], "1");
    for warning in ["1 null", "2 empty", "2 geometries"] {
        assert!(
            l.warnings.iter().any(|w| w.contains(warning)),
            "{:?}",
            l.warnings
        );
    }
}
#[test]
fn standalone_native_collections_and_empty_layers_pass_gdal_and_sqlite_checks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.gpkg");
    let mut empty = layer(vec![]);
    empty.name = "empty_violations".into();
    empty.fields = vec![Field {
        name: "violation_id".into(),
        kind: FieldType::Integer,
    }];
    let mut mixed = layer(vec![sample_geometries().pop().unwrap()]);
    mixed.records[0].id = Some(-1);
    write::dataset(&path, None, false, &[mixed, empty]).unwrap();
    sqlite_check(&path);
    let ds = Dataset::open(&path).unwrap();
    assert_eq!(ds.layer_count(), 2);
    let mut l = ds.layer_by_name("features").unwrap();
    let f = l.features().next().unwrap();
    // SAFETY: GDAL owns a live feature and exposes its signed FID.
    assert_eq!(unsafe { gdal_sys::OGR_F_GetFID(f.c_feature()) }, -1);
    assert!(matches!(
        gdal_geometry(f.geometry().unwrap()),
        Geometry::GeometryCollection(_)
    ));
    assert_eq!(
        ds.layer_by_name("empty_violations")
            .unwrap()
            .feature_count(),
        0
    );
    let copied = dir.path().join("copy.gpkg");
    std::fs::copy(&path, &copied).unwrap();
    assert_eq!(Dataset::open(&copied).unwrap().layer_count(), 2);
    sqlite_check(&copied);
}
#[test]
fn native_formats_take_precedence_and_malformed_native_files_do_not_fallback() {
    let all = registry::formats();
    for name in [
        "GeoJSON",
        "GPKG",
        "FlatGeobuf",
        "ESRI Shapefile",
        "GeoParquet",
        "KML",
        "GPX",
    ] {
        let matches = all.iter().filter(|f| f.name == name).collect::<Vec<_>>();
        assert_eq!(matches.len(), 1);
        assert!(matches[0].native);
    }
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("data.csv");
    std::fs::write(&csv, "WKT,label\n\"POINT (5 52)\",retained\n").unwrap();
    let mut l = read::read_with_attributes(&csv, None, "csv", true)
        .unwrap()
        .remove(0);
    assert_eq!(l.layer.len(), 1);
    assert_eq!(l.attributes.remove(0)["label"], "retained");
    for ext in ["geojson", "gpkg", "fgb", "parquet", "kml", "gpx"] {
        let fake = dir.path().join(format!("disguised.{ext}"));
        std::fs::copy(&csv, &fake).unwrap();
        assert!(read::read_with_attributes(&fake, None, "fake", true).is_err());
    }
}
#[test]
fn additional_csv_driver_roundtrip_and_explicit_native_driver_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("additional.csv");
    write::dataset(
        &path,
        Some("CSV"),
        false,
        &[layer(vec![Point::new(5., 52.).into()])],
    )
    .unwrap();
    let imported = read::read_with_attributes(&path, None, "csv", true).unwrap();
    assert_eq!(imported[0].layer.len(), 1);
    assert_eq!(imported[0].attributes[0]["label"], "café & <GIS>");
    let native = dir.path().join("native.gpkg");
    write::dataset(
        &native,
        Some("GPKG"),
        false,
        &[layer(vec![Point::new(5., 52.).into()])],
    )
    .unwrap();
    let ds = Dataset::open(&native).unwrap();
    let mut sql = ds
        .execute_sql(
            "SELECT count(*) FROM sqlite_master WHERE name LIKE 'rtree_%'",
            None,
            Dialect::SQLITE,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        sql.features()
            .next()
            .unwrap()
            .field_as_integer64(0)
            .unwrap(),
        Some(0),
        "explicit GPKG uses the repository writer without GDAL's default spatial index"
    );
}
#[test]
fn unknown_drivers_duplicate_ids_and_failed_overwrites_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.csv");
    std::fs::write(&path, "existing").unwrap();
    assert!(write::dataset(&path, Some("does-not-exist"), true, &[layer(vec![])]).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing");
    let mut duplicate = layer(vec![Point::new(1., 2.).into(), Point::new(3., 4.).into()]);
    duplicate.records[1].id = duplicate.records[0].id;
    assert!(write::dataset(&path, Some("CSV"), true, &[duplicate]).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "existing");
}
#[test]
fn additional_filegdb_and_mapinfo_drivers_preserve_layers_and_replace_outputs() {
    let dir = tempfile::tempdir().unwrap();
    for (driver, ext) in [("OpenFileGDB", "gdb"), ("MapInfo File", "tab")] {
        let path = dir.path().join(format!("data.{ext}"));
        let mut original = layer(vec![Point::new(5., 52.).into()]);
        original.records[0]
            .properties
            .insert("integer".into(), 42.into());
        // Let these drivers allocate their format-specific positive identities.
        original.records[0].id = None;
        let mut layers = vec![original.clone()];
        if driver == "OpenFileGDB" {
            let mut second = original.clone();
            second.name = "second".into();
            layers.push(second);
        }
        write::dataset(&path, Some(driver), false, &layers)
            .unwrap_or_else(|e| panic!("{driver}: {e:#}"));
        assert_eq!(Dataset::open(&path).unwrap().driver().short_name(), driver);
        let imported = read::read_with_attributes(&path, None, "additional", true).unwrap();
        assert_eq!(imported.len(), layers.len());
        for l in &imported {
            assert_eq!(l.layer.len(), 1);
            assert_eq!(l.attributes[0]["integer"], 42);
            assert_eq!(l.attributes[0]["label"], "café & <GIS>");
            let plain = read::read(&path, Some(&l.selector), "selected").unwrap();
            assert_eq!(
                plain[0].layer.features().next().unwrap().0,
                l.layer.features().next().unwrap().0
            );
        }
        assert!(read::read(&path, Some("does-not-exist"), "missing").is_err());
        assert!(write::dataset(&path, Some(driver), false, &layers).is_err());
        let mut bad = original.clone();
        bad.records[0].geometry = Some(Point::new(f64::INFINITY, 52.).into());
        assert!(write::dataset(&path, Some(driver), true, &[bad]).is_err());
        assert_eq!(
            read::read(&path, None, "preserved").unwrap().len(),
            layers.len()
        );
        original.records[0]
            .properties
            .insert("integer".into(), 43.into());
        write::dataset(&path, Some(driver), true, &[original]).unwrap();
        let replaced = read::read_with_attributes(&path, None, "replaced", true).unwrap();
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].attributes[0]["integer"], 43);
    }
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".topology-output-")
    }));
}

fn field_types(path: &Path) -> Vec<(String, OGRFieldType::Type)> {
    let ds = Dataset::open(path).unwrap();
    ds.layer(0)
        .unwrap()
        .defn()
        .fields()
        .map(|f| (f.name(), f.field_type()))
        .collect()
}

fn filegdb_wide_type() -> OGRFieldType::Type {
    if gdal::version::version_info("VERSION_NUM")
        .parse::<u32>()
        .unwrap()
        >= 3_090_000
    {
        OGRFieldType::OFTInteger64
    } else {
        OGRFieldType::OFTReal
    }
}

fn assert_filegdb_integer(actual: &serde_json::Value, expected: i64) {
    if filegdb_wide_type() == OGRFieldType::OFTInteger64 {
        if expected.unsigned_abs() > 9_007_199_254_740_991 {
            assert_eq!(actual, &json!(expected.to_string()));
        } else {
            assert_eq!(actual.as_i64(), Some(expected));
        }
    } else {
        assert_eq!(actual.as_f64(), Some(expected as f64));
    }
}

#[test]
fn additional_integer_fields_use_actual_ranges_and_ignore_null_or_missing_values() {
    let dir = tempfile::tempdir().unwrap();
    for (driver, ext) in [("OpenFileGDB", "gdb"), ("MapInfo File", "tab")] {
        let path = dir.path().join(format!("ranges.{ext}"));
        let properties = [
            json!({"bounded": 42, "null_only": null}),
            json!({"bounded": i32::MIN}),
            json!({"bounded": i32::MAX, "null_only": null}),
            json!({"bounded": null}),
            json!({}),
        ];
        let mut original = DatasetLayer::new(
            "ranges",
            Some(crs::parse("EPSG:4326").unwrap()),
            properties
                .iter()
                .enumerate()
                .map(|(i, p)| Record {
                    id: None,
                    geometry: Some(Point::new(5. + i as f64, 52.).into()),
                    properties: p.as_object().unwrap().clone(),
                })
                .collect(),
        );
        original.fields = ["bounded", "null_only", "missing"]
            .map(|name| Field {
                name: name.into(),
                kind: FieldType::Integer,
            })
            .into();
        write::dataset(&path, Some(driver), false, &[original]).unwrap();
        assert_eq!(
            field_types(&path),
            ["bounded", "null_only", "missing"].map(|name| (name.into(), OGRFieldType::OFTInteger))
        );
        let imported = read::read_with_attributes(&path, None, "ranges", true).unwrap();
        assert_eq!(imported[0].attributes.len(), properties.len());
        for (actual, expected) in imported[0].attributes.iter().zip(&properties) {
            // MapInfo represents unset numeric fields as zero.
            if !expected["bounded"].is_null() {
                assert_eq!(actual["bounded"], expected["bounded"], "{driver}");
            } else if driver == "OpenFileGDB" {
                assert!(actual["bounded"].is_null());
            }
            if driver == "OpenFileGDB" {
                assert!(actual["null_only"].is_null());
                assert!(actual["missing"].is_null());
            }
        }
    }

    for outlier in [
        i64::from(i32::MAX) + 1,
        i64::from(i32::MIN) - 1,
        9007199254740993,
        -9007199254740993,
    ] {
        let path = dir.path().join(format!("wide-{outlier}.gdb"));
        let values = [
            Some(42),
            Some(i64::from(i32::MAX)),
            None,
            None,
            Some(outlier),
        ];
        let original = DatasetLayer::new(
            "wide",
            Some(crs::parse("EPSG:4326").unwrap()),
            values
                .iter()
                .enumerate()
                .map(|(i, v)| Record {
                    id: None,
                    // The late outlier must be scanned even without geometry.
                    geometry: (i == 0).then(|| Point::new(5., 52.).into()),
                    properties: if i == 3 {
                        serde_json::Map::new()
                    } else {
                        json!({"integer": v}).as_object().unwrap().clone()
                    },
                })
                .collect(),
        );
        write::dataset(&path, Some("OpenFileGDB"), false, &[original]).unwrap();
        assert_eq!(
            field_types(&path),
            [("integer".into(), filegdb_wide_type())]
        );
        let ds = Dataset::open(&path).unwrap();
        let mut layer = ds.layer(0).unwrap();
        assert_eq!(layer.feature_count(), values.len() as u64);
        for (feature, expected) in layer.features().zip(values) {
            let index = feature.field_index("integer").unwrap();
            match expected {
                Some(value) if filegdb_wide_type() == OGRFieldType::OFTInteger64 => {
                    assert_eq!(feature.field_as_integer64(index).unwrap(), Some(value));
                }
                Some(value) => {
                    assert_eq!(feature.field_as_double(index).unwrap(), Some(value as f64));
                }
                None => assert!(feature.field(index).unwrap().is_none()),
            }
        }
    }
}

#[test]
fn filegdb_eager_and_disk_reports_infer_equivalent_integer_fields() {
    let dir = tempfile::tempdir().unwrap();
    let srs = crs::parse("EPSG:4326").unwrap();
    for (case, fids) in [
        ("bounded", [42, i64::from(i32::MIN), i64::from(i32::MAX)]),
        ("wide", [42, 9007199254740993, -9007199254740993]),
    ] {
        let mut topology = Topology::new(0.).unwrap();
        let lines = topology.add_layer(Layer::from_geometries("empty_lines", []));
        for (i, fid) in fids.into_iter().enumerate() {
            let points = topology.add_layer(Layer::from_features(
                format!("points_{i}"),
                [(fid, Point::new(5., 52.).into())],
            ));
            topology
                .add_rule(Rule::PointMustBeCoveredByLines, points, Some(lines))
                .unwrap();
        }
        let eager = topology.validate().unwrap();
        let stored = DiskReport::validate(&topology, Some(dir.path())).unwrap();
        assert_eq!(stored.violation_count(), fids.len());
        for layout in [ReportLayout::Complete, ReportLayout::ByFamily] {
            let a = dir.path().join(format!("eager-{case}-{layout:?}.gdb"));
            let b = dir.path().join(format!("stored-{case}-{layout:?}.gdb"));
            write::report(&a, Some("OpenFileGDB"), false, Some(&srs), &eager, layout).unwrap();
            write::stored_report(&b, Some("OpenFileGDB"), false, Some(&srs), &stored, layout)
                .unwrap();
            let schema = field_types(&a);
            assert_eq!(schema, field_types(&b));
            for (name, ty) in &schema {
                let expected = match name.as_str() {
                    "origin_fid" if case == "wide" => filegdb_wide_type(),
                    "violation_id" | "origin_fid" | "origin_part" | "destination_fid"
                    | "destination_part" => OGRFieldType::OFTInteger,
                    _ => OGRFieldType::OFTString,
                };
                assert_eq!(*ty, expected, "{case} {layout:?} {name}");
            }
            let a = read::read_with_attributes(&a, None, "report", true).unwrap();
            let b = read::read_with_attributes(&b, None, "report", true).unwrap();
            assert_eq!(a[0].attributes, b[0].attributes);
            assert_eq!(a[0].attributes.len(), fids.len());
            for (actual, fid) in a[0].attributes.iter().zip(fids) {
                if case == "wide" {
                    assert_filegdb_integer(&actual["origin_fid"], fid);
                } else {
                    assert_eq!(actual["origin_fid"].as_i64(), Some(fid));
                }
                assert!(actual["destination_fid"].is_null());
            }
        }
    }
}
#[test]
fn gdal_fallback_crs_and_invalid_coordinate_errors_are_explicit() {
    // Geocentric EPSG:4978 is outside the native horizontal CRS parser.
    let source = crs::parse("EPSG:4978").unwrap();
    assert_eq!(source.epsg(), Some(4978));
    assert!(source.to_wkt().unwrap().contains("4978"));
    let target = crs::parse("EPSG:4326").unwrap();
    let t = crs::Transform::new(&source, &target).unwrap();
    let mut x = [6378137.];
    let mut y = [0.];
    t.transform_coords(&mut x, &mut y, &mut []).unwrap();
    assert!(x[0].abs() < 1e-8 && y[0].abs() < 1e-8);
    assert!(crs::parse("EPSG:999999999").is_err());
    let native = crs::Transform::new(&target, &crs::parse("EPSG:3857").unwrap()).unwrap();
    let mut x = [5., f64::NAN];
    let mut y = [52., 52.];
    assert!(native.transform_coords(&mut x, &mut y, &mut []).is_err());
    assert_eq!(x[0], 5.);
    assert!(
        native
            .transform_coords(&mut [5.], &mut [], &mut [])
            .is_err()
    );
    assert!(native.transform_bounds(&[0., 0., 1., 1.], -1).is_err());
}
#[cfg(feature = "gdal")]
#[test]
fn bundled_proj_database_survives_invalid_external_paths_and_is_cleaned_up() {
    const WORKER: &str = "TOPOLOGY_BUNDLED_PROJ_WORKER";
    if std::env::var_os(WORKER).is_some() {
        let source = crs::parse("EPSG:4978").unwrap();
        let t = crs::Transform::new(&source, &crs::parse("EPSG:4326").unwrap()).unwrap();
        let mut x = [6378137.];
        let mut y = [0.];
        t.transform_coords(&mut x, &mut y, &mut []).unwrap();
        assert!(x[0].abs() < 1e-8 && y[0].abs() < 1e-8);
        // SAFETY: OSR returns an owned, null-terminated string list. Copy its
        // first string while live, then free the list with GDAL's allocator.
        let path = unsafe {
            let paths = gdal_sys::OSRGetPROJSearchPaths();
            assert!(!paths.is_null() && !(*paths).is_null());
            let path = std::ffi::CStr::from_ptr(*paths)
                .to_str()
                .unwrap()
                .to_string();
            gdal_sys::CSLDestroy(paths);
            path
        };
        assert!(Path::new(&path).join("proj.db").is_file());
        println!("BUNDLED_PROJ_DIRECTORY={path}");
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "bundled_proj_database_survives_invalid_external_paths_and_is_cleaned_up",
            "--nocapture",
        ])
        .env(WORKER, "1")
        .env("PROJ_DATA", "/nonexistent-topology-proj")
        .env("PROJ_LIB", "/nonexistent-topology-proj")
        .env("PROJ_NETWORK", "OFF")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let path = text
        .lines()
        .find_map(|l| l.strip_prefix("BUNDLED_PROJ_DIRECTORY="))
        .unwrap();
    assert!(
        !Path::new(path).exists(),
        "bundled PROJ temporary data is cleaned at process exit"
    );
}
#[test]
fn native_crs_transformations_and_bounds_agree_with_gdal() {
    for (from, to, input, tolerance) in [
        (4326, 3857, (5., 52.), 1e-6),
        (3857, 4326, (556597.4539663679, 6800125.454397307), 1e-8),
        (28992, 4326, (155000., 463000.), 2e-6),
        (4326, 28992, (5.38720621, 52.15517440), 0.3),
    ] {
        let source = crs::parse(&format!("EPSG:{from}")).unwrap();
        let target = crs::parse(&format!("EPSG:{to}")).unwrap();
        let native = crs::Transform::new(&source, &target).unwrap();
        let mut a = SpatialRef::from_epsg(from).unwrap();
        let mut b = SpatialRef::from_epsg(to).unwrap();
        a.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
        b.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
        let gdal = gdal::spatial_ref::CoordTransform::new(&a, &b).unwrap();
        let mut x = [input.0];
        let mut y = [input.1];
        native.transform_coords(&mut x, &mut y, &mut []).unwrap();
        let mut gx = [input.0];
        let mut gy = [input.1];
        gdal.transform_coords(&mut gx, &mut gy, &mut []).unwrap();
        assert!((x[0] - gx[0]).abs() < tolerance, "{from}->{to} X");
        assert!((y[0] - gy[0]).abs() < tolerance, "{from}->{to} Y");
        let delta = if from == 4326 { 0.001 } else { 10. };
        let bounds = [
            input.0 - delta,
            input.1 - delta,
            input.0 + delta,
            input.1 + delta,
        ];
        let n = native.transform_bounds(&bounds, 21).unwrap();
        let g = gdal.transform_bounds(&bounds, 21).unwrap();
        for i in 0..4 {
            assert!(
                (n[i] - g[i]).abs() < tolerance,
                "{from}->{to} bound {i}: {} vs {}",
                n[i],
                g[i]
            );
        }
    }
}
#[test]
fn native_shapefile_writer_and_gdal_encoding_inputs() {
    let dir = tempfile::tempdir().unwrap();
    for (i, g) in sample_geometries().into_iter().take(6).enumerate() {
        let path = dir.path().join(format!("native{i}.shp"));
        let mut l = layer(vec![g]);
        l.records[0]
            .properties
            .insert("integer".into(), 9007199254740993i64.into());
        write::dataset(&path, None, false, &[l]).unwrap();
        let ds = Dataset::open_ex(
            &path,
            gdal::DatasetOptions {
                open_options: Some(&["ADJUST_TYPE=YES"]),
                ..Default::default()
            },
        )
        .unwrap();
        let mut data = ds.layer(0).unwrap();
        assert_eq!(data.feature_count(), 1);
        let feature = data.features().next().unwrap();
        assert_eq!(
            feature
                .field_as_integer64(feature.field_index("integer").unwrap())
                .unwrap(),
            Some(9007199254740993)
        );
        assert_eq!(
            read::read_with_attributes(&path, None, "native", true).unwrap()[0].attributes[0]["integer"],
            "9007199254740993"
        );
        assert!(ds.layer(0).unwrap().spatial_ref().is_some());
    }
    let path = dir.path().join("encoded.shp");
    {
        let mut ds = DriverManager::get_driver_by_name("ESRI Shapefile")
            .unwrap()
            .create_vector_only(&path)
            .unwrap();
        // The bundled GDAL omits iconv, so create the ordinary fixture with GDAL
        // and encode the DBF bytes explicitly to exercise native CPG handling.
        let options = ["ENCODING=UTF-8"];
        let l = ds
            .create_layer(LayerOptions {
                name: "encoded",
                ty: OGRwkbGeometryType::wkbPoint,
                options: Some(&options),
                ..Default::default()
            })
            .unwrap();
        l.create_defn_fields(&[("label", OGRFieldType::OFTString)])
            .unwrap();
        let mut f = Feature::new(l.defn()).unwrap();
        f.set_geometry(Point::new(5., 52.).to_gdal().unwrap())
            .unwrap();
        f.set_field_string(0, "cafe").unwrap();
        f.create(&l).unwrap();
    }
    let dbf = path.with_extension("dbf");
    let mut bytes = std::fs::read(&dbf).unwrap();
    let index = bytes.windows(4).position(|b| b == b"cafe").unwrap();
    bytes[index + 3] = 0xe9;
    std::fs::write(dbf, bytes).unwrap();
    std::fs::write(path.with_extension("cpg"), "CP1252").unwrap();
    assert_eq!(
        read::read_with_attributes(&path, None, "encoded", true).unwrap()[0].attributes[0]["label"],
        "café"
    );
}
fn create_with_wkt(path: &Path, driver: &str, wkts: &[&str]) {
    let mut ds = DriverManager::get_driver_by_name(driver)
        .unwrap()
        .create_vector_only(path)
        .unwrap();
    let l = ds
        .create_layer(LayerOptions {
            name: "mixed",
            ty: OGRwkbGeometryType::wkbUnknown,
            ..Default::default()
        })
        .unwrap();
    l.create_defn_fields(&[
        ("label", OGRFieldType::OFTString),
        ("size", OGRFieldType::OFTReal),
        ("count", OGRFieldType::OFTInteger),
        ("big", OGRFieldType::OFTInteger64),
        ("day", OGRFieldType::OFTDate),
        ("moment", OGRFieldType::OFTDateTime),
    ])
    .unwrap();
    for (i, wkt) in wkts.iter().enumerate() {
        let mut f = Feature::new(l.defn()).unwrap();
        f.set_field_string(0, &i.to_string()).unwrap();
        f.set_field_double(1, 1.5).unwrap();
        f.set_field_integer(2, 7).unwrap();
        f.set_field_integer64(3, 9007199254740993).unwrap();
        f.set_field_string(4, "2024-05-06").unwrap();
        f.set_field_string(5, "2024-05-06 07:08:09").unwrap();
        f.set_geometry(gdal::vector::Geometry::from_wkt(wkt).unwrap())
            .unwrap();
        f.create(&l).unwrap();
    }
}
#[test]
fn additional_drivers_convert_every_geometry_kind_and_field_type() {
    let dir = tempfile::tempdir().unwrap();
    let wkts = [
        "POINT (1 2)",
        "LINESTRING (0 0, 1 1)",
        "POLYGON ((0 0, 10 0, 10 10, 0 0), (1 1, 2 1, 2 2, 1 1))",
        "MULTILINESTRING ((0 0, 1 1), (2 2, 3 3))",
        "MULTIPOLYGON (((0 0, 1 0, 1 1, 0 0)), ((5 5, 6 5, 6 6, 5 5)))",
    ];
    let path = dir.path().join("mixed.tab");
    create_with_wkt(&path, "MapInfo File", &wkts);
    let imported = read::read_with_attributes(&path, None, "mixed", true).unwrap();
    assert_eq!(imported[0].layer.len(), wkts.len());
    let first = &imported[0].attributes[0];
    assert_eq!(first["size"], 1.5);
    assert_eq!(first["count"], 7);
    assert_eq!(first["big"], "9007199254740993");
    assert_eq!(first["day"], "2024-05-06");
    assert!(
        first["moment"]
            .as_str()
            .unwrap()
            .starts_with("2024-05-06 07:08:09")
    );
}
fn wkt_csv(dir: &Path, name: &str, wkts: &[String]) -> std::path::PathBuf {
    let path = dir.join(name);
    let rows = wkts
        .iter()
        .enumerate()
        .map(|(i, wkt)| format!("\"{wkt}\",{i}\n"))
        .collect::<String>();
    std::fs::write(&path, format!("WKT,label\n{rows}")).unwrap();
    path
}
#[test]
fn wkt_csv_exercises_empty_multi_and_collection_geometries() {
    let dir = tempfile::tempdir().unwrap();
    let wkts = [
        "POINT (1 2)",
        "POINT EMPTY",
        "POLYGON EMPTY",
        "MULTIPOINT ((1 1), (2 2))",
        "MULTIPOINT (EMPTY, (3 3))",
        "GEOMETRYCOLLECTION (POINT (1 1), POINT EMPTY, LINESTRING (0 0, 1 1))",
        "GEOMETRYCOLLECTION (MULTIPOINT ((1 1)), GEOMETRYCOLLECTION (POLYGON ((0 0, 1 0, 1 1, 0 0))))",
        "POLYGON Z ((0 0 1, 10 0 1, 10 10 1, 0 0 1), (1 1 1, 2 1 1, 2 2 1, 1 1 1))",
    ]
    .map(String::from);
    let path = wkt_csv(dir.path(), "kinds.csv", &wkts);
    let imported = read::read_with_attributes(&path, None, "kinds", true)
        .unwrap()
        .remove(0);
    assert!(imported.layer.len() >= 5, "{:?}", imported.warnings);
    assert!(imported.warnings.iter().any(|w| w.contains("empty")));
    assert!(imported.warnings.iter().any(|w| w.contains("Z/M")));
}
#[test]
fn unsupported_gdal_geometry_types_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = wkt_csv(
        dir.path(),
        "curve.csv",
        &["CIRCULARSTRING (0 0, 1 1, 2 0)".to_string()],
    );
    let error = read::read(&path, None, "curve").unwrap_err();
    assert!(
        format!("{error:#}").contains("unsupported GDAL geometry"),
        "{error:#}"
    );
}

#[test]
fn fallback_coordinate_systems_expose_the_same_helpers() {
    // Geocentric EPSG:4978 is outside the native horizontal CRS parser.
    let geocentric = crs::parse("EPSG:4978").unwrap();
    assert!(!geocentric.is_geographic() && !geocentric.is_projected());
    assert_eq!(geocentric.linear_units(), 1.);
    assert_eq!(geocentric.name().as_deref(), Some("WGS 84"));
    assert!(geocentric.to_projjson().unwrap().contains("WGS 84"));
    assert_eq!(geocentric, crs::parse("EPSG:4978").unwrap());
    let geographic = crs::parse("EPSG:4326").unwrap();
    assert_ne!(geocentric, geographic);
    assert_ne!(geographic, geocentric);
    let to_geographic = crs::Transform::new(&geocentric, &geographic).unwrap();
    assert!(to_geographic.transform_bounds(&[0., 0., 1., 1.], 2).is_ok());
    // GDAL reports points outside a projection's domain as errors, not infinities.
    let orthographic = crs::parse("+proj=ortho +lat_0=0 +lon_0=0").unwrap();
    let operation = crs::Transform::new(&geographic, &orthographic).unwrap();
    assert!(
        operation
            .transform_coords(&mut [180.], &mut [0.], &mut [])
            .is_err()
    );
}
#[test]
fn vrt_over_geojson_exposes_list_date_and_nested_geometry_types() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.geojson");
    std::fs::write(
        &source,
        r#"{"type":"FeatureCollection","features":[
        {"type":"Feature","properties":{"ints":[1,2],"reals":[1.5],"names":["a","b"],"day":"2024-05-06","moment":"2024-05-06T07:08:09Z","big":9007199254740993,"bigs":[9007199254740993,2]},
         "geometry":{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,0]],[[1,1],[2,1],[2,2],[1,1]]]}},
        {"type":"Feature","properties":{"ints":[3],"reals":[2.5],"names":["c"],"day":"2024-05-07","moment":"2024-05-07T07:08:09Z","big":1,"bigs":[3]},
         "geometry":{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]}},
        {"type":"Feature","properties":{"ints":[4],"reals":[3.5],"names":["d"],"day":null,"moment":null,"big":2,"bigs":[4]},
         "geometry":{"type":"MultiLineString","coordinates":[[[0,0],[1,1]],[[2,2],[3,3]]]}},
        {"type":"Feature","properties":{"ints":[5],"reals":[4.5],"names":["e"],"day":"2024-05-08","moment":"2024-05-08T07:08:09Z","big":3,"bigs":[5]},
         "geometry":{"type":"MultiPoint","coordinates":[[1,1],[2,2]]}}]}"#,
    )
    .unwrap();
    let vrt = dir.path().join("view.vrt");
    std::fs::write(
        &vrt,
        format!(
            "<OGRVRTDataSource><OGRVRTLayer name=\"view\"><SrcDataSource>{}</SrcDataSource><SrcLayer>source</SrcLayer></OGRVRTLayer></OGRVRTDataSource>",
            source.display()
        ),
    )
    .unwrap();
    let layer = read::read_with_attributes(&vrt, None, "view", true)
        .unwrap()
        .remove(0);
    assert_eq!(layer.layer.len(), 4);
    assert_eq!(layer.attributes[0]["ints"], json!([1, 2]));
    assert_eq!(layer.attributes[0]["reals"], json!([1.5]));
    assert_eq!(layer.attributes[0]["names"], json!(["a", "b"]));
    assert!(layer.attributes[0]["day"].is_string());
    assert_eq!(layer.attributes[0]["bigs"], json!(["9007199254740993", 2]));
}
#[test]
fn vrt_without_layers_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let vrt = dir.path().join("empty.vrt");
    std::fs::write(&vrt, "<OGRVRTDataSource></OGRVRTDataSource>").unwrap();
    let error = read::read(&vrt, None, "x").unwrap_err();
    assert!(
        format!("{error:#}").contains("no vector layers"),
        "{error:#}"
    );
}

#[test]
fn reports_replace_existing_filegdb_directories_and_map_declared_geometry_types() {
    use topology_checker_core::{Layer, Rule, Topology};
    let dir = tempfile::tempdir().unwrap();
    let mut topology = Topology::new(0.).unwrap();
    let points = topology.add_layer(Layer::from_features(
        "points",
        [1, 2].map(|fid| (fid, Point::new(5., 52.).into())),
    ));
    topology
        .add_rule(Rule::PointMustNotOverlap, points, None)
        .unwrap();
    let stored = DiskReport::validate(&topology, Some(dir.path())).unwrap();
    let srs = crs::parse("EPSG:4326").unwrap();
    let path = dir.path().join("report.gdb");
    for _ in 0..2 {
        write::stored_report(
            &path,
            Some("OpenFileGDB"),
            true,
            Some(&srs),
            &stored,
            write::ReportLayout::Complete,
        )
        .unwrap();
    }
    assert!(path.is_dir());

    // Declared geometry types win over what the records would suggest.
    let line = LineString::from(vec![(5., 52.), (6., 53.)]);
    let square = polygon![(x:5.,y:52.),(x:6.,y:52.),(x:6.,y:53.),(x:5.,y:52.)];
    for (declared, geometry) in [
        (
            "MULTILINESTRING",
            MultiLineString(vec![line.clone()]).into(),
        ),
        ("MULTIPOLYGON", MultiPolygon(vec![square.clone()]).into()),
        (
            "GEOMETRYCOLLECTION",
            Geometry::GeometryCollection(GeometryCollection(vec![Point::new(5., 52.).into()])),
        ),
    ] {
        let mut declared_layer = layer(vec![geometry]);
        declared_layer.geometry_type = Some(declared.into());
        // FileGDB stores multi-part types; CSV keeps collections as text.
        let (driver, extension) = if declared == "GEOMETRYCOLLECTION" {
            ("CSV", "csv")
        } else {
            ("OpenFileGDB", "gdb")
        };
        let path = dir.path().join(format!("{declared}.{extension}"));
        write::dataset(&path, Some(driver), false, &[declared_layer])
            .unwrap_or_else(|e| panic!("{declared} via {driver}: {e:#}"));
        assert!(
            !read::read(&path, None, "x").unwrap().is_empty(),
            "{declared}"
        );
    }
    // Mixed record types fall back to an unknown layer type.
    let mixed = layer(vec![Point::new(5., 52.).into(), line.into()]);
    let path = dir.path().join("mixed.csv");
    write::dataset(&path, Some("CSV"), false, &[mixed]).unwrap();
    assert!(!read::read(&path, None, "x").unwrap().is_empty());
}
