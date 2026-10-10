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
