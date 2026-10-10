use assert_cmd::{Command, cargo::cargo_bin_cmd};
use gdal::{Dataset, vector::LayerAccess};
use std::path::Path;
use tempfile::TempDir;

fn cli() -> Command {
    cargo_bin_cmd!("topology-checker")
}
fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .display()
        .to_string()
}
#[test]
fn registry_and_driver_commands_work() {
    let output = cli()
        .args(["--json", "rules"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rules: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(rules.as_array().unwrap().len(), 18);
    cli()
        .args(["drivers", "--read", "--write"])
        .assert()
        .success();
}
#[test]
fn exit_codes_and_no_output_for_clean_data() {
    let temp = TempDir::new().unwrap();
    let output = temp.path().join("clean.gpkg");
    cli()
        .args([
            "geometry",
            "must-not-be-multipart",
            &fixture("points.geojson"),
            "-o",
        ])
        .arg(&output)
        .assert()
        .success();
    assert!(!output.exists());
    cli()
        .args(["point", "must-not-overlap", &fixture("points.geojson")])
        .assert()
        .code(1);
    cli()
        .args(["point", "must-not-overlap", "missing.geojson"])
        .assert()
        .code(2);
    cli()
        .args(["polygon", "must-not-overlap", &fixture("points.geojson")])
        .assert()
        .code(2);
}
#[test]
fn geojson_output_is_deterministic_and_overwrite_is_explicit() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("errors.geojson");
    for _ in 0..12 {
        cli()
            .args([
                "point",
                "must-not-overlap",
                &fixture("points.geojson"),
                "--overwrite",
                "-o",
            ])
            .arg(&path)
            .assert()
            .code(1);
        let ds = Dataset::open(&path).unwrap();
        assert_eq!(ds.driver().short_name(), "GeoJSON");
    }
    let original = std::fs::read(&path).unwrap();
    cli()
        .args([
            "point",
            "must-not-overlap",
            &fixture("points.geojson"),
            "-o",
        ])
        .arg(&path)
        .assert()
        .code(2);
    assert_eq!(original, std::fs::read(&path).unwrap());
}
#[test]
fn gpkg_preserves_feature_ids_fields_and_crs() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("errors.gpkg");
    cli()
        .args([
            "point",
            "must-not-overlap",
            &fixture("points.geojson"),
            "-o",
        ])
        .arg(&path)
        .assert()
        .code(1);
    let ds = Dataset::open(&path).unwrap();
    let mut layer = ds.layer_by_name("errors_point").unwrap();
    assert_eq!(layer.feature_count(), 1);
    assert!(layer.spatial_ref().is_some());
    let fields = [
        "rule",
        "error_type",
        "origin_layer",
        "origin_fid",
        "destination_layer",
        "destination_fid",
    ]
    .map(|n| layer.defn().field_index(n).unwrap());
    let f = layer.features().next().unwrap();
    assert_eq!(
        f.field_as_string(fields[0]).unwrap().unwrap(),
        "point/must-not-overlap"
    );
    assert_eq!(f.field_as_integer64(fields[3]).unwrap(), Some(41));
    assert_eq!(f.field_as_integer64(fields[5]).unwrap(), Some(92));
}
#[test]
fn toml_paths_are_relative_and_cli_tolerance_wins() {
    let temp = TempDir::new().unwrap();
    std::fs::copy(
        fixture("points.geojson"),
        temp.path().join("points.geojson"),
    )
    .unwrap();
    let config = temp.path().join("topology.toml");
    std::fs::write(&config,"tolerance = 0.25\n[layers]\npoints = { path = 'points.geojson' }\n[[rules]]\nrule = 'point/must-not-overlap'\nlayer = 'points'\n").unwrap();
    let output = cli()
        .args(["--json", "--tolerance", "0", "validate"])
        .arg(&config)
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let report: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(report["tolerance"], 0.);
    assert_eq!(report["rules"][0]["layer"], "points");
}
#[test]
fn null_empty_and_z_geometries_are_handled() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("nulls.geojson");
    std::fs::write(&input,r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":null},{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[]}},{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[0,0,1],[1,0,2]]}}]}"#).unwrap();
    let output = cli()
        .args(["--json", "geometry", "must-not-be-multipart"])
        .arg(&input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let warnings = report["warnings"].to_string();
    assert!(warnings.contains("null"));
    assert!(warnings.contains("empty"));
    assert!(warnings.contains("Z/M"));
}

#[test]
fn merge_and_explode_utilities_write_expected_features() {
    let temp = TempDir::new().unwrap();
    for (utility, count, layer) in [
        ("merge-lines", 1, "merged_line"),
        ("explode-lines", 2, "exploded_line"),
    ] {
        let path = temp.path().join(format!("{utility}.gpkg"));
        cli()
            .args(["utilities", utility, &fixture("lines.geojson"), "-o"])
            .arg(&path)
            .assert()
            .success();
        let ds = Dataset::open(path).unwrap();
        assert_eq!(ds.layer_by_name(layer).unwrap().feature_count(), count);
    }
}
#[test]
fn line_errors_include_feature_pairs_and_mixed_output_files() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("mixed.geojson");
    std::fs::write(&input,r#"{"type":"FeatureCollection","features":[{"type":"Feature","id":1,"properties":{},"geometry":{"type":"LineString","coordinates":[[0,0],[2,0]]}},{"type":"Feature","id":2,"properties":{},"geometry":{"type":"LineString","coordinates":[[1,0],[3,0],[3,-1],[0.5,-1],[0.5,1]]}}]}"#).unwrap();
    let path = temp.path().join("errors.geojson");
    cli()
        .args(["line", "must-not-intersect"])
        .arg(&input)
        .arg("-o")
        .arg(&path)
        .assert()
        .code(1);
    assert!(!path.exists());
    for kind in ["line", "point"] {
        let ds = Dataset::open(temp.path().join(format!("errors_{kind}.geojson"))).unwrap();
        assert_eq!(ds.driver().short_name(), "GeoJSON");
    }
    let gpkg = temp.path().join("errors.gpkg");
    cli()
        .args(["line", "must-not-intersect"])
        .arg(&input)
        .arg("-o")
        .arg(&gpkg)
        .assert()
        .code(1);
    let ds = Dataset::open(gpkg).unwrap();
    assert!(ds.layer_by_name("errors_line").is_ok());
    assert!(ds.layer_by_name("errors_point").is_ok());
}
#[test]
fn explicit_driver_is_honored_and_invalid_tolerances_fail_cleanly() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("output.json");
    cli()
        .args([
            "--driver",
            "GPKG",
            "point",
            "must-not-overlap",
            &fixture("points.geojson"),
            "-o",
        ])
        .arg(&path)
        .assert()
        .code(1);
    assert_eq!(Dataset::open(path).unwrap().driver().short_name(), "GPKG");
    for tolerance in ["-1", "NaN", "inf"] {
        cli()
            .args([
                "--tolerance",
                tolerance,
                "point",
                "must-not-overlap",
                &fixture("points.geojson"),
            ])
            .assert()
            .code(2);
    }
}

fn gpkg(path: &Path, layers: &[(&str, &str)], epsg: u32) {
    use gdal::{
        DriverManager,
        spatial_ref::SpatialRef,
        vector::{Geometry, LayerOptions},
    };
    let mut ds = DriverManager::get_driver_by_name("GPKG")
        .unwrap()
        .create_vector_only(path)
        .unwrap();
    let srs = SpatialRef::from_epsg(epsg).unwrap();
    for &(name, wkt) in layers {
        let geometry = Geometry::from_wkt(wkt).unwrap();
        let mut layer = ds
            .create_layer(LayerOptions {
                name,
                srs: Some(&srs),
                ty: geometry.geometry_type(),
                ..Default::default()
            })
            .unwrap();
        layer.create_feature(geometry).unwrap();
    }
    ds.flush_cache().unwrap();
}
fn json_report(assertion: assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&assertion.get_output().stdout).unwrap()
}
#[test]
fn physical_layers_are_separate_selectable_and_binary_rules_expand_pairs() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a.gpkg");
    let b = temp.path().join("b.gpkg");
    gpkg(
        &a,
        &[
            ("first", "LINESTRING (0 0, 1 0)"),
            ("second", "LINESTRING (1 0, 2 0)"),
        ],
        3857,
    );
    gpkg(
        &b,
        &[
            ("first", "LINESTRING (0 0, 2 0)"),
            ("second", "LINESTRING (0 0, 2 0)"),
        ],
        3857,
    );
    let report = json_report(
        cli()
            .args(["--json", "line", "must-not-have-dangles"])
            .arg(&a)
            .assert()
            .code(1),
    );
    assert_eq!(report["rules"].as_array().unwrap().len(), 2);
    assert!(
        report["rules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["errors"].as_array().unwrap().len() == 2)
    );
    let selected = json_report(
        cli()
            .args(["--json", "line", "must-not-have-dangles"])
            .arg(&a)
            .args(["--layer", "second"])
            .assert()
            .code(1),
    );
    assert_eq!(selected["rules"].as_array().unwrap().len(), 1);
    let cross = json_report(
        cli()
            .args(["--json", "line", "must-not-overlap-with"])
            .arg(&a)
            .arg("--other")
            .arg(&b)
            .assert()
            .code(1),
    );
    assert_eq!(cross["rules"].as_array().unwrap().len(), 4);
    let cross = json_report(
        cli()
            .args(["--json", "line", "must-not-overlap-with"])
            .arg(&a)
            .args(["--layer", "first", "--other"])
            .arg(&b)
            .args(["--other-layer", "second"])
            .assert()
            .code(1),
    );
    assert_eq!(cross["rules"].as_array().unwrap().len(), 1);
    cli()
        .args(["line", "must-not-have-dangles"])
        .arg(&a)
        .args(["--layer", "missing"])
        .assert()
        .code(2);
}
#[test]
fn crs_defaults_units_mismatch_and_wrong_kind_have_clear_results() {
    let temp = TempDir::new().unwrap();
    for (epsg, expected) in [(3857, 0.001), (4326, 1e-8), (2263, 0.001 / (1200. / 3937.))] {
        let path = temp.path().join(format!("crs{epsg}.gpkg"));
        gpkg(&path, &[("points", "POINT (0 0)")], epsg);
        let report = json_report(
            cli()
                .args(["--json", "point", "must-not-overlap"])
                .arg(&path)
                .assert()
                .success(),
        );
        assert!((report["tolerance"].as_f64().unwrap() - expected).abs() < expected * 1e-10);
    }
    let a = temp.path().join("crs3857.gpkg");
    let b = temp.path().join("crs4326.gpkg");
    let output = cli()
        .args(["point", "must-not-overlap-with"])
        .arg(&a)
        .arg("--other")
        .arg(&b)
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&output).contains("different CRSs"));
    let output = cli()
        .args(["line", "must-not-have-dangles"])
        .arg(&a)
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("FID 1") && text.contains("expected Line"));
}
#[test]
fn validation_writes_three_geometry_families_and_nullable_gap_identity() {
    let temp = TempDir::new().unwrap();
    gpkg(
        &temp.path().join("inputs.gpkg"),
        &[
            ("roads", "LINESTRING (0 0, 1 0)"),
            ("parcels", "POLYGON ((0 0, 2 0, 2 2, 0 2, 0 0))"),
            ("others", "POLYGON ((1 0, 3 0, 3 2, 1 2, 1 0))"),
        ],
        3857,
    );
    let config = temp.path().join("topology.toml");
    std::fs::write(&config, "[layers]\nroads = { path = 'inputs.gpkg', layer = 'roads' }\nparcels = { path = 'inputs.gpkg', layer = 'parcels' }\nothers = { path = 'inputs.gpkg', layer = 'others' }\n[[rules]]\nrule = 'line/must-not-have-dangles'\nlayer = 'roads'\n[[rules]]\nrule = 'polygon/must-not-have-gaps'\nlayer = 'parcels'\n[[rules]]\nrule = 'polygon/must-not-overlap-with'\nlayer = 'parcels'\nother = 'others'\n").unwrap();
    let path = temp.path().join("errors.gpkg");
    cli()
        .arg("validate")
        .arg(&config)
        .arg("-o")
        .arg(&path)
        .assert()
        .code(1);
    let ds = Dataset::open(&path).unwrap();
    assert_eq!(ds.layer_count(), 3);
    for (name, count) in [
        ("errors_point", 2),
        ("errors_line", 1),
        ("errors_polygon", 1),
    ] {
        let layer = ds.layer_by_name(name).unwrap();
        assert_eq!(layer.feature_count(), count);
        assert_eq!(layer.spatial_ref().unwrap().auth_code().unwrap(), 3857);
    }
    let mut layer = ds.layer_by_name("errors_line").unwrap();
    let fid = layer.defn().field_index("origin_fid").unwrap();
    assert!(
        layer
            .features()
            .next()
            .unwrap()
            .field(fid)
            .unwrap()
            .is_none()
    );
}
#[test]
fn shapefile_aliases_and_unknown_extensions_are_handled() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("errors.shp");
    let report = json_report(
        cli()
            .args([
                "--json",
                "point",
                "must-not-overlap",
                &fixture("points.geojson"),
                "-o",
            ])
            .arg(&path)
            .assert()
            .code(1),
    );
    assert!(report["warnings"].to_string().contains("orig_layer"));
    let ds = Dataset::open(&path).unwrap();
    let layer = ds.layer(0).unwrap();
    for name in ["orig_layer", "origin_fid", "dest_layer", "dest_fid"] {
        assert!(layer.defn().field_index(name).is_ok());
    }
    cli()
        .args([
            "point",
            "must-not-overlap",
            &fixture("points.geojson"),
            "-o",
        ])
        .arg(temp.path().join("errors.unknown"))
        .assert()
        .code(2);
    cli()
        .args([
            "--driver",
            "VRT",
            "point",
            "must-not-overlap",
            &fixture("points.geojson"),
            "-o",
        ])
        .arg(temp.path().join("errors.vrt"))
        .assert()
        .code(2);
    // An explicit driver takes priority over the extension, including the warning text.
    let report = json_report(
        cli()
            .args([
                "--json",
                "--driver",
                "GPKG",
                "point",
                "must-not-overlap",
                &fixture("points.geojson"),
                "-o",
            ])
            .arg(temp.path().join("actual-gpkg.shp"))
            .assert()
            .code(1),
    );
    assert!(!report["warnings"].to_string().contains("orig_layer"));
}
#[test]
fn malformed_configs_fail_without_panicking() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("topology.toml");
    for content in [
        "[layers]\n",
        "[layers]\np = { path = 'points.geojson' }\n[[rules]]\nrule = 'point/must-not-overlap'\nlayer = 'missing'\n",
        "[layers]\np = { path = 'points.geojson' }\n[[rules]]\nrule = 'point/must-not-overlap-with'\nlayer = 'p'\n",
    ] {
        std::fs::write(&path, content).unwrap();
        cli().arg("validate").arg(&path).assert().code(2);
    }
}

#[test]
fn split_outputs_are_all_checked_before_any_file_is_created() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("input.geojson");
    std::fs::write(&input, r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[0,0],[2,0]]}},{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[1,0],[3,0],[3,-1],[0.5,-1],[0.5,1]]}}]}"#).unwrap();
    let point = temp.path().join("errors_point.geojson");
    std::fs::write(&point, "preserve this file").unwrap();
    cli()
        .args(["line", "must-not-intersect"])
        .arg(&input)
        .arg("-o")
        .arg(temp.path().join("errors.geojson"))
        .assert()
        .code(2);
    assert_eq!(
        std::fs::read_to_string(point).unwrap(),
        "preserve this file"
    );
    assert!(!temp.path().join("errors_line.geojson").exists());
}
#[test]
fn empty_collection_children_do_not_become_phantom_points() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("input.gpkg");
    gpkg(
        &path,
        &[(
            "collection",
            "GEOMETRYCOLLECTION (POINT EMPTY, POINT (2 3))",
        )],
        3857,
    );
    cli()
        .args(["geometry", "must-not-be-multipart"])
        .arg(&path)
        .assert()
        .success();
}
#[test]
fn missing_crs_defaults_to_exact_and_degenerate_utilities_warn() {
    use gdal::{
        DriverManager,
        vector::{Geometry, LayerOptions},
    };
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("points.shp");
    let mut ds = DriverManager::get_driver_by_name("ESRI Shapefile")
        .unwrap()
        .create_vector_only(&path)
        .unwrap();
    let mut layer = ds
        .create_layer(LayerOptions {
            ty: gdal_sys::OGRwkbGeometryType::wkbPoint,
            ..Default::default()
        })
        .unwrap();
    layer
        .create_feature(Geometry::from_wkt("POINT (0 0)").unwrap())
        .unwrap();
    drop(ds);
    let report = json_report(
        cli()
            .args(["--json", "point", "must-not-overlap"])
            .arg(&path)
            .assert()
            .success(),
    );
    assert_eq!(report["tolerance"], 0.);
    assert!(report["warnings"].to_string().contains("missing CRS"));
    let input = temp.path().join("line.geojson");
    std::fs::write(&input, r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[0,0],[0,0]]}}]}"#).unwrap();
    let report = json_report(
        cli()
            .args(["--json", "utilities", "merge-lines"])
            .arg(&input)
            .arg("-o")
            .arg(temp.path().join("empty.gpkg"))
            .assert()
            .success(),
    );
    assert_eq!(report["features"], 0);
    assert!(report["warnings"].to_string().contains("degenerate"));
}
#[test]
fn clean_polygon_binary_and_empty_gap_runs_create_no_output() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a.gpkg");
    let b = temp.path().join("b.gpkg");
    gpkg(&a, &[("area", "POLYGON ((0 0, 1 0, 1 1, 0 1, 0 0))")], 3857);
    gpkg(&b, &[("area", "POLYGON ((2 0, 3 0, 3 1, 2 1, 2 0))")], 3857);
    let output = temp.path().join("no-errors.gpkg");
    cli()
        .args(["polygon", "must-not-overlap-with"])
        .arg(&a)
        .arg("--other")
        .arg(&b)
        .arg("-o")
        .arg(&output)
        .assert()
        .success();
    assert!(!output.exists());
    let empty = temp.path().join("empty.geojson");
    std::fs::write(&empty, r#"{"type":"FeatureCollection","features":[]}"#).unwrap();
    cli()
        .args(["polygon", "must-not-have-gaps"])
        .arg(empty)
        .arg("-o")
        .arg(&output)
        .assert()
        .success();
    assert!(!output.exists());
}

#[test]
fn negative_gdal_feature_ids_are_preserved() {
    use gdal::{DatasetOptions, GdalOpenFlags, vector::sql::Dialect};
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("negative.gpkg");
    gpkg(&path, &[("roads", "LINESTRING (0 0, 1 0)")], 3857);
    let ds = Dataset::open_ex(
        &path,
        DatasetOptions {
            open_flags: GdalOpenFlags::GDAL_OF_VECTOR | GdalOpenFlags::GDAL_OF_UPDATE,
            ..Default::default()
        },
    )
    .unwrap();
    ds.execute_sql("UPDATE roads SET fid = -41", None, Dialect::DEFAULT)
        .unwrap();
    drop(ds);
    let report = json_report(
        cli()
            .args(["--json", "line", "must-not-have-dangles"])
            .arg(&path)
            .assert()
            .code(1),
    );
    let errors = report["rules"][0]["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2);
    assert!(errors.iter().all(|e| e["origin"]["fid"] == -41));
}

#[test]
fn generated_ids_do_not_collide_with_real_negative_ids() {
    use gdal::{DatasetOptions, GdalOpenFlags, vector::sql::Dialect};
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("ids.gpkg");
    gpkg(&path, &[("roads", "LINESTRING (0 0, 1 0)")], 3857);
    let ds = Dataset::open_ex(
        &path,
        DatasetOptions {
            open_flags: GdalOpenFlags::GDAL_OF_VECTOR | GdalOpenFlags::GDAL_OF_UPDATE,
            ..Default::default()
        },
    )
    .unwrap();
    let mut layer = ds.layer_by_name("roads").unwrap();
    layer
        .create_feature(gdal::vector::Geometry::from_wkt("LINESTRING (2 0, 3 0)").unwrap())
        .unwrap();
    ds.execute_sql("UPDATE roads SET fid = -fid", None, Dialect::DEFAULT)
        .unwrap();
    drop(ds);
    let report = json_report(
        cli()
            .args(["--json", "line", "must-not-have-dangles"])
            .arg(&path)
            .assert()
            .code(1),
    );
    let errors = report["rules"][0]["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 4);
    let ids = errors
        .iter()
        .map(|e| e["origin"]["fid"].as_i64().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(ids.contains(&-2));
    assert_eq!(ids.len(), 2);
    assert!(
        report["warnings"]
            .to_string()
            .contains("assigned negative FIDs")
    );
}
