use assert_cmd::cargo::cargo_bin_cmd;
use geo::{LineString, Point};
use serde_json::{Map, Value, json};
use std::ffi::OsStr;
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
/// A definition of one rule on a line with two ends that touch nothing, in a
/// folder, with the path of the output.
fn definition(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let layer = DatasetLayer::new(
        "line",
        Some(crs::parse("EPSG:4326").unwrap()),
        vec![Record {
            id: Some(1),
            geometry: Some(LineString::from(vec![(5., 52.), (6., 53.)]).into()),
            properties: Map::new(),
        }],
    );
    write::dataset(&dir.join("lines.geojson"), None, false, &[layer]).unwrap();
    let config = dir.join("topology.toml");
    std::fs::write(
        &config,
        "[layers]\nroads = { path = \"lines.geojson\" }\n\n[[rules]]\nrule = \"line/must-not-have-dangles\"\nlayer = \"roads\"\n",
    )
    .unwrap();
    (config, dir.join("errors.gpkg"))
}
#[test]
fn summary_counts_the_violations_without_listing_them_and_still_writes_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let (config, output) = definition(dir.path());
    let full = cargo_bin_cmd!("topology-checker")
        .args(["--json", "validate"])
        .arg(&config)
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let full: Value = serde_json::from_slice(&full).unwrap();
    assert_eq!(full["rules"][0]["errors"].as_array().unwrap().len(), 2);
    let short = cargo_bin_cmd!("topology-checker")
        .args(["--json", "--summary", "validate"])
        .arg(&config)
        .arg("-o")
        .arg(&output)
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    assert!(short.len() < full_len(&full));
    let short: Value = serde_json::from_slice(&short).unwrap();
    let rule = &short["rules"][0];
    assert!(rule.get("errors").is_none());
    assert_eq!(rule["counts"], full["rules"][0]["counts"]);
    assert_eq!(rule["spec"], full["rules"][0]["spec"]);
    let errors = read::read_with_attributes(&output, None, "errors", true).unwrap();
    assert_eq!(errors[0].layer.len(), 2);
}
fn full_len(report: &Value) -> usize {
    serde_json::to_vec(report).unwrap().len()
}
#[test]
fn summary_without_json_prints_the_same_table() {
    let dir = tempfile::tempdir().unwrap();
    let (config, _) = definition(dir.path());
    let plain = cargo_bin_cmd!("topology-checker")
        .arg("validate")
        .arg(&config)
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let summary = cargo_bin_cmd!("topology-checker")
        .args(["--summary", "validate"])
        .arg(&config)
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    assert_eq!(plain, summary);
    assert!(
        String::from_utf8(plain)
            .unwrap()
            .contains("topology errors")
    );
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

#[test]
fn usage_and_input_errors_exit_with_status_two() {
    let dir = tempfile::tempdir().unwrap();
    let points = dir.path().join("points.geojson");
    std::fs::write(
        &points,
        json!({"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[5,52]}}]}).to_string(),
    )
    .unwrap();
    let missing = dir.path().join("missing.geojson");
    let usage = cargo_bin_cmd!("topology-checker")
        .arg("--no-such-flag")
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    assert!(String::from_utf8_lossy(&usage).contains("--no-such-flag"));
    cargo_bin_cmd!("topology-checker")
        .args(["point", "must-be-covered-by-lines"])
        .arg(&points)
        .arg("--other")
        .arg(&missing)
        .assert()
        .code(2);
    cargo_bin_cmd!("topology-checker")
        .args(["utilities", "merge-lines"])
        .arg(&missing)
        .arg("-o")
        .arg(dir.path().join("merged.geojson"))
        .assert()
        .code(2);
}

#[test]
fn utilities_refuse_to_replace_existing_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("lines.geojson");
    let layer = DatasetLayer::new(
        "lines",
        Some(crs::parse("EPSG:4326").unwrap()),
        vec![Record {
            id: Some(1),
            geometry: Some(LineString::from(vec![(5., 52.), (6., 53.)]).into()),
            properties: Map::new(),
        }],
    );
    write::dataset(&input, None, false, &[layer]).unwrap();
    let output = dir.path().join("merged.geojson");
    std::fs::write(&output, "keep").unwrap();
    cargo_bin_cmd!("topology-checker")
        .args(["utilities", "merge-lines"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .assert()
        .code(2);
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "keep");
}

/// GeoJSON layers for the progress tests: a line with two dangles and a
/// degenerate part, two coincident points, and two polygons whose gap cannot
/// be emitted exactly.
fn progress_inputs(dir: &std::path::Path) {
    let x = 1f64.next_up();
    for (name, features) in [
        (
            "lines",
            json!([
                {"type":"Feature","id":1,"properties":{},"geometry":{"type":"LineString","coordinates":[[5,52],[6,53]]}},
                {"type":"Feature","id":2,"properties":{},"geometry":{"type":"LineString","coordinates":[[5,53],[5,53]]}}
            ]),
        ),
        (
            "points",
            json!([
                {"type":"Feature","id":1,"properties":{},"geometry":{"type":"Point","coordinates":[5,52]}},
                {"type":"Feature","id":2,"properties":{},"geometry":{"type":"Point","coordinates":[5,52]}}
            ]),
        ),
        (
            "narrow",
            json!([
                {"type":"Feature","id":-17,"properties":{},"geometry":{"type":"Polygon","coordinates":[[[1.,0.],[x,0.],[x,1.],[1.,1.],[1.,0.]]]}},
                {"type":"Feature","id":82,"properties":{},"geometry":{"type":"Polygon","coordinates":[[[1.,-1.],[x,-1.],[1.,1.],[1.,-1.]]]}}
            ]),
        ),
    ] {
        let collection = json!({"type":"FeatureCollection","features":features});
        std::fs::write(dir.join(format!("{name}.geojson")), collection.to_string()).unwrap();
    }
}

/// A definition with the given `(rule, layer alias)` checks.
fn progress_config(dir: &std::path::Path, rules: &[(&str, &str)]) -> std::path::PathBuf {
    let mut text = String::from(if rules.is_empty() { "rules = []\n" } else { "" });
    text.push_str(
        "[layers]\nroads = { path = \"lines.geojson\" }\nwells = { path = \"points.geojson\" }\nparcels = { path = \"narrow.geojson\" }\n",
    );
    for (rule, layer) in rules {
        text.push_str(&format!(
            "\n[[rules]]\nrule = \"{rule}\"\nlayer = \"{layer}\"\n"
        ));
    }
    let path = dir.join("topology.toml");
    std::fs::write(&path, text).unwrap();
    path
}

const THREE_RULES: [(&str, &str); 3] = [
    ("line/must-not-have-dangles", "roads"),
    ("line/must-not-self-overlap", "roads"),
    ("point/must-not-overlap", "wells"),
];

fn validate(args: &[&str], config: &std::path::Path, extra: &[&OsStr]) -> std::process::Output {
    let mut command = cargo_bin_cmd!("topology-checker");
    command.args(args).arg("validate").arg(config).args(extra);
    command.output().unwrap()
}

fn progress_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|l| l.starts_with("checked rule "))
        .collect()
}

const PROGRESS: &str = "--progress";

#[test]
fn progress_prints_one_line_per_finished_rule_before_the_warnings() {
    let dir = tempfile::tempdir().unwrap();
    progress_inputs(dir.path());
    let config = progress_config(dir.path(), &THREE_RULES);
    let progress = validate(&[], &config, &[OsStr::new(PROGRESS)]);
    assert_eq!(progress.status.code(), Some(1));
    let stderr = String::from_utf8(progress.stderr).unwrap();
    let lines = stderr.lines().collect::<Vec<_>>();
    assert_eq!(
        lines[..3],
        [
            "checked rule 1/3 (line/must-not-have-dangles): 2 errors",
            "checked rule 2/3 (line/must-not-self-overlap): 0 errors",
            "checked rule 3/3 (point/must-not-overlap): 1 error",
        ],
        "{stderr}"
    );
    // Warnings still come last, after every rule has finished.
    assert!(lines.len() > 3, "{stderr}");
    assert!(
        lines[3..].iter().all(|l| l.starts_with("warning: ")),
        "{stderr}"
    );
    assert!(stderr.contains("degenerate"), "{stderr}");
    let plain = validate(&[], &config, &[]);
    assert_eq!(plain.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(plain.stderr).unwrap(),
        lines[3..].join("\n") + "\n"
    );
    // Each line names the rule and count the report gives at that position.
    let report = validate(&["--json"], &config, &[OsStr::new(PROGRESS)]);
    let json: Value = serde_json::from_slice(&report.stdout).unwrap();
    let stderr = String::from_utf8(report.stderr).unwrap();
    let rules = json["rules"].as_array().unwrap();
    let progress = progress_lines(&stderr);
    assert_eq!(progress.len(), rules.len());
    for (i, (line, rule)) in progress.iter().zip(rules).enumerate() {
        let count = rule["errors"].as_array().unwrap().len();
        let unit = if count == 1 { "error" } else { "errors" };
        let name = rule["spec"]["rule"].as_str().unwrap();
        let expected = format!(
            "checked rule {}/{} ({name}): {count} {unit}",
            i + 1,
            rules.len()
        );
        assert_eq!(*line, expected);
    }
}

#[test]
fn progress_counts_every_registered_rule_of_multi_layer_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let layer = |name: &str, x: f64| {
        DatasetLayer::new(
            name,
            Some(crs::parse("EPSG:4326").unwrap()),
            vec![Record {
                id: Some(1),
                geometry: Some(LineString::from(vec![(x, 52.), (x + 1., 53.)]).into()),
                properties: Map::new(),
            }],
        )
    };
    write::dataset(
        &dir.path().join("network.gpkg"),
        None,
        false,
        &[layer("east", 5.), layer("west", 9.)],
    )
    .unwrap();
    let config = dir.path().join("topology.toml");
    std::fs::write(
        &config,
        "[layers]\nnetwork = { path = \"network.gpkg\" }\n\n[[rules]]\nrule = \"line/must-not-have-dangles\"\nlayer = \"network\"\n\n[[rules]]\nrule = \"line/must-not-self-overlap\"\nlayer = \"network\"\n",
    )
    .unwrap();
    let output = validate(&[], &config, &[OsStr::new(PROGRESS)]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        progress_lines(&stderr),
        [
            "checked rule 1/4 (line/must-not-have-dangles): 2 errors",
            "checked rule 2/4 (line/must-not-have-dangles): 2 errors",
            "checked rule 3/4 (line/must-not-self-overlap): 0 errors",
            "checked rule 4/4 (line/must-not-self-overlap): 0 errors",
        ],
        "{stderr}"
    );
}

/// Every file in a folder, by name, with its bytes.
fn folder(path: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut files = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

/// The features and attributes of every readable dataset in a folder.
fn decoded(path: &std::path::Path) -> Vec<String> {
    let mut layers = Vec::new();
    for (name, _) in folder(path) {
        let file = path.join(&name);
        if topology_checker_io::registry::for_path(&file).is_none() {
            continue;
        }
        for layer in read::read_with_attributes(&file, None, "errors", true).unwrap() {
            let features = layer.layer.features().collect::<Vec<_>>();
            layers.push(format!(
                "{name} {} {features:?} {:?}",
                layer.selector, layer.attributes
            ));
        }
    }
    layers
}

#[test]
fn progress_leaves_stdout_and_outputs_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    progress_inputs(dir.path());
    let config = progress_config(dir.path(), &THREE_RULES);
    for mode in [
        &[][..],
        &["--json"],
        &["--json", "--summary"],
        &["--summary"],
    ] {
        let plain = validate(mode, &config, &[]);
        let progress = validate(mode, &config, &[OsStr::new(PROGRESS)]);
        assert_eq!(plain.status.code(), Some(1));
        assert_eq!(progress.status.code(), Some(1));
        assert!(!plain.stdout.is_empty());
        assert_eq!(plain.stdout, progress.stdout, "{mode:?}");
    }
    // Error reports group points into multipoints, which GPX refuses with or
    // without progress.
    for case in [
        "geojson", "gpkg", "fgb", "parquet", "kml", "kmz", "gpx", "shp",
    ] {
        let run = |name: &str, progress: bool| {
            let folder = dir.path().join(format!("{case}-{name}"));
            std::fs::create_dir(&folder).unwrap();
            let output = folder.join(format!("errors.{case}"));
            let mut extra = vec![OsStr::new("-o"), output.as_os_str()];
            extra.extend(progress.then(|| OsStr::new(PROGRESS)));
            let result = validate(&["--json"], &config, &extra);
            let stderr = String::from_utf8(result.stderr).unwrap();
            (result.status.code(), result.stdout, stderr, folder)
        };
        let (code, plain, plain_err, plain_folder) = run("plain", false);
        let (again_code, again, _, again_folder) = run("again", false);
        let (progress_code, progress, progress_err, progress_folder) = run("progress", true);
        assert_eq!(
            [again_code, progress_code],
            [code; 2],
            "{case}: {plain_err}"
        );
        assert_eq!(plain, again, "{case}");
        assert_eq!(plain, progress, "{case}");
        // Progress only adds its own lines to stderr.
        let rest = progress_err
            .lines()
            .filter(|l| !l.starts_with("checked rule "))
            .collect::<Vec<_>>();
        assert_eq!(rest, plain_err.lines().collect::<Vec<_>>(), "{case}");
        assert_eq!(progress_lines(&progress_err).len(), 3, "{progress_err}");
        if case == "gpx" {
            assert_eq!(code, Some(2), "{plain_err}");
            assert!(plain_err.contains("GPX export supports"), "{plain_err}");
            assert!(folder(&plain_folder).is_empty());
            assert!(folder(&progress_folder).is_empty());
            continue;
        }
        assert_eq!(code, Some(1), "{case}: {plain_err}");
        assert!(!folder(&plain_folder).is_empty(), "{case}");
        if folder(&plain_folder) == folder(&again_folder) {
            assert_eq!(folder(&plain_folder), folder(&progress_folder), "{case}");
        } else {
            // Formats that stamp the write time differ run to run; compare contents.
            let names = |f: &std::path::Path| {
                folder(f)
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>()
            };
            assert_eq!(names(&plain_folder), names(&progress_folder), "{case}");
            assert!(!decoded(&plain_folder).is_empty(), "{case}");
            assert_eq!(decoded(&plain_folder), decoded(&progress_folder), "{case}");
        }
    }
}

#[test]
fn progress_needs_a_rule_to_report() {
    let dir = tempfile::tempdir().unwrap();
    progress_inputs(dir.path());
    let config = progress_config(dir.path(), &[]);
    let output = validate(&[], &config, &[OsStr::new(PROGRESS)]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("no rules"), "{stderr}");
    assert!(progress_lines(&stderr).is_empty(), "{stderr}");
    assert!(output.stdout.is_empty());
}

#[test]
fn progress_stops_before_the_rule_that_fails() {
    let dir = tempfile::tempdir().unwrap();
    progress_inputs(dir.path());
    let config = progress_config(
        dir.path(),
        &[
            ("line/must-not-have-dangles", "roads"),
            ("polygon/must-not-have-gaps", "parcels"),
            ("point/must-not-overlap", "wells"),
        ],
    );
    let output = dir.path().join("errors.geojson");
    let result = validate(
        &[],
        &config,
        &[
            OsStr::new("--progress"),
            OsStr::new("-o"),
            output.as_os_str(),
        ],
    );
    assert!(!result.status.success());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert_eq!(
        progress_lines(&stderr),
        ["checked rule 1/3 (line/must-not-have-dangles): 2 errors"],
        "{stderr}"
    );
    assert!(stderr.contains("precision failure"), "{stderr}");
    assert!(result.stdout.is_empty());
    assert!(!output.exists());
}

/// Squares meeting along their edges: no violations, and seconds of overlap
/// checks in debug and release builds alike.
fn touching_squares(path: &std::path::Path) {
    use std::fmt::Write;
    let side = if cfg!(debug_assertions) { 120 } else { 400 };
    let mut text = String::from(r#"{"type":"FeatureCollection","features":["#);
    for k in 0..side * side {
        let (x, y) = (k % side, k / side);
        let (x1, y1) = (x + 1, y + 1);
        if k > 0 {
            text.push(',');
        }
        write!(
            text,
            r#"{{"type":"Feature","id":{k},"properties":{{}},"geometry":{{"type":"Polygon","coordinates":[[[{x},{y}],[{x1},{y}],[{x1},{y1}],[{x},{y1}],[{x},{y}]]]}}}}"#
        )
        .unwrap();
    }
    text.push_str("]}");
    std::fs::write(path, text).unwrap();
}

#[test]
fn progress_lines_arrive_while_later_rules_still_run() {
    use std::io::BufRead;
    let dir = tempfile::tempdir().unwrap();
    progress_inputs(dir.path());
    touching_squares(&dir.path().join("squares.geojson"));
    let config = dir.path().join("topology.toml");
    std::fs::write(
        &config,
        "[layers]\nroads = { path = \"lines.geojson\" }\nsquares = { path = \"squares.geojson\" }\n\n[[rules]]\nrule = \"line/must-not-have-dangles\"\nlayer = \"roads\"\n\n[[rules]]\nrule = \"polygon/must-not-overlap\"\nlayer = \"squares\"\n",
    )
    .unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_topology-checker"))
        .arg("validate")
        .arg(&config)
        .arg(PROGRESS)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut line = || {
        let mut line = String::new();
        stderr.read_line(&mut line).unwrap();
        (line, std::time::Instant::now())
    };
    let (first, first_at) = line();
    let running = child.try_wait().unwrap().is_none();
    let (second, second_at) = line();
    let status = child.wait().unwrap();
    assert_eq!(
        [first.as_str(), second.as_str()],
        [
            "checked rule 1/2 (line/must-not-have-dangles): 2 errors\n",
            "checked rule 2/2 (polygon/must-not-overlap): 0 errors\n",
        ]
    );
    assert_eq!(status.code(), Some(1));
    // Checking the squares takes seconds; load only makes that longer. Lines
    // held back until the end would arrive together.
    assert!(running);
    let gap = second_at - first_at;
    assert!(gap.as_secs_f64() > 0.5, "the lines came {gap:?} apart");
}
