use assert_cmd::cargo::cargo_bin_cmd;
use geo::{LineString, Point};
use serde_json::{Map, Value, json};
use topology_checker_io::{
    crs, read,
    write::{self, DatasetLayer, Record},
};
#[test]
fn default_cli_lists_native_formats_and_validates_every_native_format() {
    let output = cargo_bin_cmd!("topology-checker")
        .args(["--json", "drivers", "--read", "--write"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let formats: Value = serde_json::from_slice(&output).unwrap();
    for name in [
        "GeoJSON",
        "GPKG",
        "ESRI Shapefile",
        "FlatGeobuf",
        "GeoParquet",
        "KML",
        "GPX",
    ] {
        assert!(
            formats
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["name"] == name)
        );
    }
    let dir = tempfile::tempdir().unwrap();
    for ext in [
        "geojson", "gpkg", "fgb", "parquet", "kml", "kmz", "gpx", "shp",
    ] {
        let input = dir.path().join(format!("line.{ext}"));
        let output = dir.path().join(format!("errors-{ext}.gpkg"));
        let layer = DatasetLayer::new(
            "line",
            Some(crs::parse("EPSG:4326").unwrap()),
            vec![Record {
                id: Some(-1),
                geometry: Some(LineString::from(vec![(5., 52.), (6., 53.)]).into()),
                properties: Map::new(),
            }],
        );
        write::dataset(&input, None, false, &[layer]).unwrap();
        cargo_bin_cmd!("topology-checker")
            .arg("--json")
            .args(["line", "must-not-have-dangles"])
            .arg(input)
            .arg("-o")
            .arg(&output)
            .assert()
            .code(1);
        let errors = read::read_with_attributes(&output, None, "errors", true).unwrap();
        assert_eq!(errors[0].layer.len(), 2, "{ext}");
        assert_eq!(
            errors[0].attributes[0]["rule"],
            "line/must-not-have-dangles"
        );
    }
}
#[test]
fn native_cli_preserves_signed_ids_and_rejects_duplicate_ids() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("ids.geojson");
    let output = dir.path().join("errors.geojson");
    let collection = json!({"type":"FeatureCollection","features":[{"type":"Feature","id":-1,"properties":{},"geometry":{"type":"Point","coordinates":[5,52]}},{"type":"Feature","id":9007199254740993i64,"properties":{},"geometry":{"type":"Point","coordinates":[5,52]}}]});
    std::fs::write(&input, collection.to_string()).unwrap();
    let report = cargo_bin_cmd!("topology-checker")
        .args(["--json", "point", "must-not-overlap"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let report: Value = serde_json::from_slice(&report).unwrap();
    assert!(report.to_string().contains("9007199254740993"));
    let l = read::read_with_attributes(&output, None, "errors", true).unwrap();
    assert_eq!(l[0].attributes[0]["origin_fid"], -1);
    cargo_bin_cmd!("topology-checker")
        .args(["point", "must-not-overlap"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .assert()
        .code(2);
    let duplicate = DatasetLayer::new(
        "duplicate",
        None,
        vec![
            Record {
                id: Some(-1),
                geometry: Some(Point::new(5., 52.).into()),
                properties: Map::new(),
            },
            Record {
                id: Some(-1),
                geometry: Some(Point::new(5., 52.).into()),
                properties: Map::new(),
            },
        ],
    );
    assert!(
        write::dataset(
            &dir.path().join("duplicate.gpkg"),
            None,
            false,
            &[duplicate]
        )
        .is_err()
    );
}
#[test]
fn reports_use_the_shared_columns_and_shapefile_short_names() {
    let dir = tempfile::tempdir().unwrap();
    let points = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/points.geojson");
    let gpkg = dir.path().join("errors.gpkg");
    cargo_bin_cmd!("topology-checker")
        .args(["point", "must-not-overlap", points, "-o"])
        .arg(&gpkg)
        .assert()
        .code(1);
    let layer = read::read_with_attributes(&gpkg, Some("errors_point"), "errors", true)
        .unwrap()
        .remove(0);
    let row = &layer.attributes[0];
    let mut columns = row.keys().map(String::as_str).collect::<Vec<_>>();
    columns.sort_unstable();
    assert_eq!(
        columns,
        [
            "destination_fid",
            "destination_layer",
            "destination_part",
            "error_type",
            "origin_fid",
            "origin_layer",
            "origin_part",
            "rule",
            "violation_id",
        ]
    );
    assert_eq!(row["violation_id"], 0);
    assert_eq!(row["rule"], "point/must-not-overlap");
    assert_eq!(row["origin_fid"], 41);
    assert_eq!(row["destination_fid"], 92);
    assert_eq!(row["origin_part"], 0);
    let shp = dir.path().join("errors.shp");
    let output = cargo_bin_cmd!("topology-checker")
        .args(["--json", "point", "must-not-overlap", points, "-o"])
        .arg(&shp)
        .assert()
        .code(1)
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&output).contains("viol_id"));
    let layer = read::read_with_attributes(&shp, None, "errors", true)
        .unwrap()
        .remove(0);
    for name in [
        "viol_id",
        "orig_layer",
        "origin_fid",
        "orig_part",
        "dest_layer",
        "dest_fid",
        "dest_part",
    ] {
        assert!(layer.attributes[0].contains_key(name), "{name}");
    }
}

#[test]
fn precision_failure_never_publishes_an_export_or_successful_summary() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("precision.geojson");
    let output = dir.path().join("errors.geojson");
    let x = 1f64.next_up();
    let data = json!({"type":"FeatureCollection","features":[
        {"type":"Feature","id":-17,"properties":{},"geometry":{"type":"Polygon","coordinates":[[[1.,0.],[x,0.],[x,1.],[1.,1.],[1.,0.]]]}},
        {"type":"Feature","id":82,"properties":{},"geometry":{"type":"Polygon","coordinates":[[[1.,-1.],[x,-1.],[1.,1.],[1.,-1.]]]}}
    ]});
    std::fs::write(&input, data.to_string()).unwrap();
    let result = cargo_bin_cmd!("topology-checker")
        .args(["--json", "polygon", "must-not-have-gaps"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(String::from_utf8_lossy(&result.stderr).contains("precision failure"));
    assert!(result.stdout.is_empty());
    assert!(!output.exists());
    std::fs::write(&output, "keep existing output").unwrap();
    cargo_bin_cmd!("topology-checker")
        .args(["polygon", "must-not-have-gaps"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("--overwrite")
        .assert()
        .failure();
    assert_eq!(
        std::fs::read_to_string(output).unwrap(),
        "keep existing output"
    );
}
