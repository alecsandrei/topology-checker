#![cfg(feature = "gdal-system")]

use assert_cmd::{Command, cargo::cargo_bin_cmd};
use gdal::{Dataset, vector::LayerAccess};
use std::path::Path;
use tempfile::TempDir;
use topology_checker_core::{GeometryKind, Rule};

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
    assert_eq!(rules.as_array().unwrap().len(), 38);
    let expected = Rule::ALL
        .iter()
        .map(|rule| {
            format!(
                "{{\"name\":{},\"description\":{},\"arcgis\":{},\"qgis\":{},\"layer\":{},\"other\":{}}}",
                serde_json::to_string(rule.name()).unwrap(),
                serde_json::to_string(rule.description()).unwrap(),
                serde_json::to_string(&rule.arcgis()).unwrap(),
                serde_json::to_string(&rule.qgis()).unwrap(),
                serde_json::to_string(&rule.inputs().0).unwrap(),
                serde_json::to_string(&rule.inputs().1).unwrap(),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(output, format!("[{expected}]\n").as_bytes());
    cli()
        .args(["drivers", "--read", "--write"])
        .assert()
        .success();
}

#[test]
fn rules_catalog_groups_every_rule_with_its_required_inputs() {
    let output = cli()
        .arg("rules")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    assert!(text.is_ascii());
    assert!(text.ends_with('\n'));
    assert!(text.lines().all(|line| line.len() <= 80));
    let mut blocks = text.split("\n\n");
    assert_eq!(blocks.next(), Some("Available topology rules (38)"));
    let mut names = Vec::new();
    for (kind, heading) in [
        (GeometryKind::Point, "Points (10)"),
        (GeometryKind::Line, "Lines (19)"),
        (GeometryKind::Polygon, "Polygons (7)"),
        (GeometryKind::Any, "All geometries (2)"),
    ] {
        assert_eq!(blocks.next(), Some(heading));
        for &rule in Rule::ALL.iter().filter(|rule| rule.inputs().0 == kind) {
            let block = blocks.next().unwrap();
            let lines = block.lines().collect::<Vec<_>>();
            assert_eq!(lines[0], format!("  {}", rule.name()));
            names.push(lines[0].trim());
            assert!(lines.len() >= 3);
            assert!(
                lines[1..lines.len() - 1]
                    .iter()
                    .all(|line| { line.starts_with("    ") && !line.trim().is_empty() })
            );
            let needs = match rule.inputs() {
                (GeometryKind::Point, None) => "one point layer",
                (GeometryKind::Point, Some(GeometryKind::Line)) => "point layer + line layer",
                (GeometryKind::Point, Some(GeometryKind::Point)) => "two point layers",
                (GeometryKind::Point, Some(GeometryKind::Polygon)) => "point layer + polygon layer",
                (GeometryKind::Line, Some(GeometryKind::Point)) => "line layer + point layer",
                (GeometryKind::Line, None) => "one line layer",
                (GeometryKind::Line, Some(GeometryKind::Line)) => "two line layers",
                (GeometryKind::Line, Some(GeometryKind::Polygon)) => "line layer + polygon layer",
                (GeometryKind::Polygon, Some(GeometryKind::Line)) => "polygon layer + line layer",
                (GeometryKind::Polygon, Some(GeometryKind::Point)) => "polygon layer + point layer",
                (GeometryKind::Polygon, None) => "one polygon layer",
                (GeometryKind::Polygon, Some(GeometryKind::Polygon)) => "two polygon layers",
                (GeometryKind::Any, Some(GeometryKind::Any)) => "two layers of any geometry type",
                (GeometryKind::Any, None) => "one layer of any geometry type",
                inputs => panic!("unexpected registry inputs: {inputs:?}"),
            };
            assert_eq!(*lines.last().unwrap(), format!("    Needs: {needs}"));
        }
    }
    assert_eq!(
        names,
        [
            GeometryKind::Point,
            GeometryKind::Line,
            GeometryKind::Polygon,
            GeometryKind::Any
        ]
        .into_iter()
        .flat_map(|kind| Rule::ALL
            .iter()
            .filter(move |rule| rule.inputs().0 == kind)
            .map(|rule| rule.name()))
        .collect::<Vec<_>>()
    );
    assert_eq!(
        blocks.next(),
        Some(concat!(
            "Usage: topology-checker <group> <rule> <input>\n",
            "For two layers, add: --other <input>\n",
            "Rule help: topology-checker <group> <rule> --help\n"
        ))
    );
    assert!(blocks.next().is_none());
}

#[test]
fn help_retains_clap_color_behavior_and_plain_redirected_format() {
    let plain = cli()
        .arg("--help")
        .env_remove("CLICOLOR_FORCE")
        .env("NO_COLOR", "1")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(!plain.contains(&0x1b));
    let colored = cli()
        .arg("--help")
        .env_remove("NO_COLOR")
        .env("CLICOLOR_FORCE", "1")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(colored.contains(&0x1b));
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
fn summary_leaves_the_violations_out_of_the_json_report() {
    let args = [
        "--json",
        "point",
        "must-not-overlap",
        &fixture("points.geojson"),
    ];
    let full = json_report(cli().args(args).assert().code(1));
    let rules = full["rules"].as_array().unwrap();
    assert!(!rules[0]["errors"].as_array().unwrap().is_empty());
    let short = json_report(cli().args(args).arg("--summary").assert().code(1));
    let summary = short["rules"].as_array().unwrap();
    assert_eq!(summary.len(), rules.len());
    for (a, b) in summary.iter().zip(rules) {
        assert!(a.get("errors").is_none());
        assert_eq!(a["counts"], b["counts"]);
        assert_eq!(a["spec"], b["spec"]);
    }
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
    for name in [
        "viol_id",
        "orig_layer",
        "origin_fid",
        "orig_part",
        "dest_layer",
        "dest_fid",
        "dest_part",
    ] {
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
        "rules = []\n[layers]\np = { path = 'points.geojson' }\n",
        "[layers]\np = { path = 'points.geojson' }\n[[rules]]\nrule = 'point/must-not-overlap'\nlayer = 'missing'\n",
        "[layers]\np = { path = 'points.geojson' }\n[[rules]]\nrule = 'point/must-not-overlap-with'\nlayer = 'p'\nother = 'missing'\n",
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
    let output = temp.path().join("errors.gpkg");
    let report = json_report(
        cli()
            .args(["--json", "line", "must-not-have-dangles"])
            .arg(&path)
            .arg("-o")
            .arg(&output)
            .assert()
            .code(1),
    );
    let errors = report["rules"][0]["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2);
    assert!(errors.iter().all(|e| e["origin"]["fid"] == -41));
    let ds = Dataset::open(&output).unwrap();
    let mut layer = ds.layer(0).unwrap();
    let origin = layer.defn().field_index("origin_fid").unwrap();
    let destination = layer.defn().field_index("destination_fid").unwrap();
    assert_eq!(
        layer.defn().fields().map(|f| f.name()).collect::<Vec<_>>(),
        [
            "violation_id",
            "rule",
            "error_type",
            "origin_layer",
            "origin_fid",
            "origin_part",
            "destination_layer",
            "destination_fid",
            "destination_part"
        ]
    );
    assert_eq!(
        layer.defn().fields().nth(origin).unwrap().field_type(),
        gdal::vector::OGRFieldType::OFTInteger64
    );
    for feature in layer.features() {
        assert_eq!(feature.field_as_integer64(origin).unwrap(), Some(-41));
        assert!(feature.field(destination).unwrap().is_none());
    }
}

#[cfg(unix)]
#[test]
fn closed_stdout_before_execution_is_graceful_and_utility_files_are_complete() {
    use std::{
        os::{fd::OwnedFd, unix::net::UnixStream},
        process::{Command, Stdio},
    };
    let temp = TempDir::new().unwrap();
    let config = temp.path().join("topology.toml");
    std::fs::write(&config, format!("[layers]\np = {{ path = '{}' }}\n[[rules]]\nrule = 'point/must-not-overlap'\nlayer = 'p'\n", fixture("points.geojson"))).unwrap();
    for json in [false, true] {
        let mut commands = vec![
            vec!["--help".into()],
            vec!["rules".into()],
            vec!["drivers".into()],
            vec![
                "point".into(),
                "must-not-overlap".into(),
                fixture("points.geojson"),
            ],
            vec!["validate".into(), config.display().to_string()],
        ];
        let mut outputs = Vec::new();
        for utility in ["merge-lines", "explode-lines"] {
            let path = temp.path().join(format!("{utility}-{json}.gpkg"));
            commands.push(vec![
                "utilities".into(),
                utility.into(),
                fixture("lines.geojson"),
                "-o".into(),
                path.display().to_string(),
            ]);
            outputs.push((utility, path));
        }
        for args in commands {
            let (reader, writer) = UnixStream::pair().unwrap();
            drop(reader);
            let fd: OwnedFd = writer.into();
            let mut child = Command::new(env!("CARGO_BIN_EXE_topology-checker"));
            if json {
                child.arg("--json");
            }
            let result = child
                .args(args)
                .stdout(Stdio::from(fd))
                .stderr(Stdio::piped())
                .output()
                .unwrap();
            assert_eq!(
                result.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(!String::from_utf8_lossy(&result.stderr).contains("error:"));
            assert!(!String::from_utf8_lossy(&result.stderr).contains("panicked"));
        }
        for (utility, path) in outputs {
            let ds = Dataset::open(path).unwrap();
            let mut layer = ds.layer(0).unwrap();
            assert_eq!(
                layer.feature_count(),
                if utility == "merge-lines" { 1 } else { 2 }
            );
            for (i, feature) in layer.features().enumerate() {
                let geometry = feature.geometry().unwrap();
                assert_eq!(geometry.geometry_count(), 1);
                let line = geometry.get_geometry(0);
                let expected = if utility == "merge-lines" {
                    vec![(0., 0.), (1., 0.), (2., 0.)]
                } else {
                    vec![(i as f64, 0.), (i as f64 + 1., 0.)]
                };
                assert_eq!(line.point_count(), expected.len());
                for (j, &(x, y)) in expected.iter().enumerate() {
                    assert_eq!(line.get_point(j as i32), (x, y, 0.));
                }
            }
        }
    }
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
    assert_eq!(ids, [-2, -1].into_iter().collect());
    assert!(
        !report["warnings"]
            .to_string()
            .contains("assigned negative FIDs")
    );
}

#[test]
fn every_new_rule_executes_through_cli_and_toml_with_identical_json() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("inputs.gpkg");
    gpkg(
        &input,
        &[
            ("points", "POINT (1 1)"),
            ("points_other", "POINT (1 1)"),
            ("lines", "LINESTRING (-1 1, 11 1)"),
            ("lines_other", "LINESTRING (0 0, 10 0)"),
            ("polygons", "POLYGON ((0 0, 10 0, 10 10, 0 10, 0 0))"),
            ("polygons_other", "POLYGON ((0 0, 10 0, 10 10, 0 10, 0 0))"),
        ],
        3857,
    );
    let name = |kind| match kind {
        GeometryKind::Point => "points",
        GeometryKind::Line => "lines",
        _ => "polygons",
    };
    for &rule in &Rule::ALL[18..] {
        let (group, command) = rule.name().split_once('/').unwrap();
        let (a, b) = rule.inputs();
        let origin = name(a);
        let destination = format!("{}_other", name(b.unwrap()));
        let direct = cli()
            .args(["--json", "--tolerance", "0", group, command])
            .arg(&input)
            .args(["--layer", origin, "--other"])
            .arg(&input)
            .args(["--other-layer", &destination])
            .output()
            .unwrap();
        assert!(
            matches!(direct.status.code(), Some(0 | 1)),
            "{rule}: {}",
            String::from_utf8_lossy(&direct.stderr)
        );
        let direct: serde_json::Value = serde_json::from_slice(&direct.stdout).unwrap();
        let config = temp.path().join("check.toml");
        std::fs::write(&config,format!("tolerance = 0\n[layers]\n{origin} = {{ path = 'inputs.gpkg', layer = '{origin}' }}\n{destination} = {{ path = 'inputs.gpkg', layer = '{destination}' }}\n[[rules]]\nrule = '{rule}'\nlayer = '{origin}'\nother = '{destination}'\n")).unwrap();
        let configured = cli()
            .args(["--json", "validate"])
            .arg(config)
            .output()
            .unwrap();
        assert!(
            matches!(configured.status.code(), Some(0 | 1)),
            "{rule}: {}",
            String::from_utf8_lossy(&configured.stderr)
        );
        let mut configured: serde_json::Value = serde_json::from_slice(&configured.stdout).unwrap();
        for error in configured["rules"][0]["errors"].as_array_mut().unwrap() {
            for (role, input) in [("origin", "layer"), ("destination", "other")] {
                if !error[role].is_null() {
                    error[role]["layer"] = direct["rules"][0]["spec"][input].clone();
                }
            }
        }
        assert_eq!(
            direct["rules"][0]["errors"], configured["rules"][0]["errors"],
            "{rule}"
        );
        assert_eq!(direct["rules"][0]["spec"]["rule"], rule.name());
    }
}

#[test]
fn new_rule_exports_keep_mixed_contacts_and_secondary_point_provenance() {
    let temp = TempDir::new().unwrap();
    gpkg(
        &temp.path().join("inputs.gpkg"),
        &[
            ("lines", "MULTILINESTRING ((1 1, 2 1), (-1 1, 0 0, 1 -1))"),
            ("polygons", "POLYGON ((0 0, 10 0, 10 10, 0 10, 0 0))"),
            ("points", "POINT (20 20)"),
        ],
        3857,
    );
    let config = temp.path().join("check.toml");
    std::fs::write(&config,"tolerance = 0\n[layers]\nlines = { path = 'inputs.gpkg', layer = 'lines' }\npolygons = { path = 'inputs.gpkg', layer = 'polygons' }\npoints = { path = 'inputs.gpkg', layer = 'points' }\n[[rules]]\nrule = 'line/must-not-intersect-polygons'\nlayer = 'lines'\nother = 'polygons'\n[[rules]]\nrule = 'polygon/must-contain-one-point'\nlayer = 'polygons'\nother = 'points'\n").unwrap();
    let output = temp.path().join("errors.gpkg");
    let report = json_report(
        cli()
            .args(["--json", "validate"])
            .arg(&config)
            .arg("-o")
            .arg(&output)
            .assert()
            .code(1),
    );
    assert_eq!(
        report["rules"][0]["errors"][0]["error_type"],
        "intersection"
    );
    assert!(
        report["rules"][0]["errors"][0]["geometry"]
            .get("GeometryCollection")
            .is_some()
    );
    assert_eq!(report["rules"][1]["errors"].as_array().unwrap().len(), 2);
    let ds = Dataset::open(output).unwrap();
    for (name, count) in [
        ("errors_line", 1),
        ("errors_point", 2),
        ("errors_polygon", 1),
    ] {
        assert_eq!(ds.layer_by_name(name).unwrap().feature_count(), count);
    }
    let mut points = ds.layer_by_name("errors_point").unwrap();
    let kind = points.defn().field_index("error_type").unwrap();
    let origin = points.defn().field_index("origin_fid").unwrap();
    let destination = points.defn().field_index("destination_fid").unwrap();
    let orphan = points
        .features()
        .find(|f| f.field_as_string(kind).unwrap().as_deref() == Some("point-count"))
        .unwrap();
    assert!(orphan.field(origin).unwrap().is_none());
    assert_eq!(orphan.field_as_integer64(destination).unwrap(), Some(1));
}
