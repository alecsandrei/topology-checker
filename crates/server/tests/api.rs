use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use topology_checker_web::{AppState, api};
use tower::ServiceExt;

async fn call(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn multipart(app: &Router, path: &str, name: &str, bytes: &[u8]) -> (StatusCode, Vec<u8>) {
    let mut body=format!("--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
    body.extend(bytes);
    body.extend(b"\r\n--BOUNDARY--\r\n");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("Content-Type", "multipart/form-data; boundary=BOUNDARY")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    (
        response.status(),
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}
async fn completed(app: &Router, id: &str) -> Value {
    for _ in 0..500 {
        let (_, r) = call(app, "GET", &format!("/workspaces/{id}/job"), Value::Null).await;
        if r["job"]["status"] != "running" {
            return r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("validation did not complete")
}
#[tokio::test]
async fn registry_formats_export_import_and_project_reopen() {
    let app = api(AppState::default());
    let (status, formats) = call(&app, "GET", "/formats", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let formats = formats.as_array().unwrap();
    for name in [
        "GeoJSON",
        "GPKG",
        "FlatGeobuf",
        "GeoParquet",
        "KML",
        "GPX",
        "ESRI Shapefile",
    ] {
        assert!(
            formats
                .iter()
                .any(|f| f["name"] == name && f["native"] == true)
        );
    }
    let (_, ws) = call(&app, "POST", "/example", Value::Null).await;
    let id = ws["id"].as_str().unwrap();
    call(&app, "POST", &format!("/workspaces/{id}/runs"), Value::Null).await;
    let result = completed(&app, id).await;
    let count = result["result"]["total"].as_u64().unwrap() as usize;
    let dir = tempfile::tempdir().unwrap();
    for ext in ["geojson", "gpkg", "fgb", "parquet", "kml", "kmz", "gpx"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/workspaces/{id}/export/{ext}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{ext}");
        assert!(
            response.headers()["content-disposition"]
                .to_str()
                .unwrap()
                .contains(&format!("violations.{ext}"))
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let path = dir.path().join(format!("violations.{ext}"));
        std::fs::write(&path, &bytes).unwrap();
        let layers =
            topology_checker_io::read::read_with_attributes(&path, None, "export", true).unwrap();
        assert_eq!(
            layers[0].layer.len(),
            count,
            "{ext}: one complete geometry per violation"
        );
        if ["geojson", "kml", "kmz", "gpx"].contains(&ext) {
            assert_eq!(
                layers[0].srs,
                Some(topology_checker_io::crs::parse("EPSG:4326").unwrap())
            );
        }
        let (_, empty) = call(&app, "POST", "/workspaces", Value::Null).await;
        let new_id = empty["id"].as_str().unwrap();
        let (status, imported) = multipart(
            &app,
            &format!("/workspaces/{new_id}/uploads"),
            &format!("violations.{ext}"),
            &bytes,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{ext}: {}",
            String::from_utf8_lossy(&imported)
        );
        let imported: Value = serde_json::from_slice(&imported).unwrap();
        assert_eq!(imported["layers"].as_array().unwrap().len(), 1);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/workspaces/{new_id}/project"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let (status, reopened) = multipart(&app, "/projects/open", "project.zip", &bytes).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{ext}: {}",
            String::from_utf8_lossy(&reopened)
        );
        let reopened: Value = serde_json::from_slice(&reopened).unwrap();
        assert_eq!(reopened["layers"], imported["layers"], "{ext}");
        assert_eq!(reopened["config"], imported["config"], "{ext}");
    }
}
#[tokio::test]
async fn example_validation_matches_core_and_keeps_signed_ids() {
    let app = api(AppState::default());
    let (status, ws) = call(&app, "POST", "/example", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let id = ws["id"].as_str().unwrap();
    let (status, _) = call(&app, "POST", &format!("/workspaces/{id}/runs"), Value::Null).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let complete = completed(&app, id).await;
    assert_eq!(complete["job"]["status"], "completed");
    let data = topology_checker_io::read::read(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/example.geojson")),
        None,
        "direct",
    )
    .unwrap()
    .remove(0);
    let mut direct = topology_checker_core::Topology::new(0.001).unwrap();
    let layer = direct.add_layer(data.layer);
    direct
        .add_rule(
            topology_checker_core::Rule::LineMustNotHaveDangles,
            layer,
            None,
        )
        .unwrap();
    direct
        .add_rule(topology_checker_core::Rule::LineMustNotOverlap, layer, None)
        .unwrap();
    let report = direct.validate().unwrap();
    assert_eq!(complete["result"]["total"], report.violation_count());
    let (_, page) = call(
        &app,
        "GET",
        &format!("/workspaces/{id}/results"),
        Value::Null,
    )
    .await;
    assert_eq!(
        page["violations"].as_array().unwrap().len(),
        report.violation_count()
    );
    assert!(
        page["violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["origin"]["fid"] == "-12")
    );
    assert!(
        page["violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["origin"]["fid"] == "9007199254740993"
                || v["destination"]["fid"] == "9007199254740993")
    );
    let layer = ws["layers"][0]["id"].as_str().unwrap();
    let (_, feature) = call(
        &app,
        "GET",
        &format!("/workspaces/{id}/layers/{layer}/features/9007199254740993"),
        Value::Null,
    )
    .await;
    assert_eq!(feature["id"], "9007199254740993");
    assert_eq!(feature["properties"]["name"], "Duplicate segment");
    let (_, viewport) = call(
        &app,
        "GET",
        &format!("/workspaces/{id}/layers/{layer}/features?bbox=154000,462000,156000,464000"),
        Value::Null,
    )
    .await;
    assert_eq!(viewport["features"].as_array().unwrap().len(), 4);
    let mut config = ws["config"].clone();
    config["tolerance"] = json!(2.);
    let (status, _) = call(&app, "PUT", &format!("/workspaces/{id}/config"), config).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/workspaces/{id}/results"),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut config = ws["config"].clone();
    config["working_crs"] = json!("EPSG:4326");
    let (status, reprojected) =
        call(&app, "PUT", &format!("/workspaces/{id}/config"), config).await;
    assert_eq!(status, StatusCode::OK, "{reprojected}");
    assert_eq!(reprojected["config"]["tolerance"], 1e-8);
}
#[tokio::test]
async fn projects_roundtrip_configuration_original_data_and_styles() {
    let app = api(AppState::default());
    let (_, ws) = call(&app, "POST", "/example", Value::Null).await;
    let id = ws["id"].as_str().unwrap();
    let mut config = ws["config"].clone();
    config["layers"][0]["color"] = json!("#bb6655");
    config["tolerance"] = json!(0.02);
    config["view"] = json!({"basemap":false,"center":[155070.,463050.],"resolution":0.5});
    let (_, ws) = call(&app, "PUT", &format!("/workspaces/{id}/config"), config).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/workspaces/{id}/project"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let (status, reopened) =
        multipart(&app, "/projects/open", "test.topology-project.zip", &bytes).await;
    let reopened: Value = serde_json::from_slice(&reopened).unwrap();
    assert_eq!(status, StatusCode::OK, "{reopened}");
    assert_eq!(ws["config"], reopened["config"]);
    assert_eq!(ws["layers"], reopened["layers"]);
    assert!(reopened["result"].is_null());
}

#[tokio::test]
async fn failed_upload_keeps_current_results_and_successful_uploads_have_distinct_colors() {
    let app = api(AppState::default());
    let (_, ws) = call(&app, "POST", "/example", Value::Null).await;
    let id = ws["id"].as_str().unwrap();
    call(&app, "POST", &format!("/workspaces/{id}/runs"), Value::Null).await;
    completed(&app, id).await;
    let (_, before) = call(&app, "GET", &format!("/workspaces/{id}"), Value::Null).await;
    let (status, _) = multipart(
        &app,
        &format!("/workspaces/{id}/uploads"),
        "bad.geojson",
        b"invalid data",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, after) = call(&app, "GET", &format!("/workspaces/{id}"), Value::Null).await;
    assert_eq!(before, after);

    let data = br#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[5,52]},"properties":{}}]}"#;
    let (status, bytes) = multipart(
        &app,
        &format!("/workspaces/{id}/uploads"),
        "points.geojson",
        data,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let after: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(after["layers"].as_array().unwrap().len(), 2);
    assert_ne!(
        after["config"]["layers"][0]["color"],
        after["config"]["layers"][1]["color"]
    );
    assert_eq!(after["config"]["layers"][1]["selected"], false);
    assert_eq!(after["config"]["checks"], before["config"]["checks"]);
    assert!(after["result"].is_null());
}
#[tokio::test]
async fn failed_jobs_clean_reports_and_pagination() {
    let app = api(AppState::default());
    let (_, ws) = call(&app, "POST", "/workspaces", Value::Null).await;
    let id = ws["id"].as_str().unwrap();
    call(&app, "POST", &format!("/workspaces/{id}/runs"), Value::Null).await;
    assert_eq!(completed(&app, id).await["job"]["status"], "failed");
    let features=(0..121).map(|i|json!({"type":"Feature","id":i,"properties":{},"geometry":{"type":"Point","coordinates":[i as f64,0]}})).collect::<Vec<_>>();
    let data = json!({"type":"FeatureCollection","features":features}).to_string();
    let (status, bytes) = multipart(
        &app,
        &format!("/workspaces/{id}/uploads"),
        "points.geojson",
        data.as_bytes(),
    )
    .await;
    let ws: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{ws}");
    let layer = ws["layers"][0]["id"].as_str().unwrap();
    let mut config = ws["config"].clone();
    config["checks"] =
        json!([{"rule":"point/must-not-overlap","layer":layer,"other":null,"enabled":true}]);
    call(
        &app,
        "PUT",
        &format!("/workspaces/{id}/config"),
        config.clone(),
    )
    .await;
    call(&app, "POST", &format!("/workspaces/{id}/runs"), Value::Null).await;
    assert_eq!(completed(&app, id).await["result"]["valid"], true);
    config["checks"] = json!([{"rule":"geometry/must-not-be-multipart","layer":layer,"other":null,"enabled":true}]);
    assert_eq!(
        call(&app, "PUT", &format!("/workspaces/{id}/config"), config)
            .await
            .0,
        StatusCode::OK
    );
    let (_, page) = call(
        &app,
        "GET",
        &format!("/workspaces/{id}/results?page=9999999999999999999"),
        Value::Null,
    )
    .await;
    assert!(page["error"].is_string());
}
#[tokio::test]
async fn local_api_rejects_foreign_origins_and_hosts() {
    let app = api(AppState::default());
    for (header, value) in [
        ("origin", "https://example.com"),
        ("host", "attacker.example:8080"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header(header, value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
#[tokio::test]
async fn local_origin_checks_follow_the_listening_port() {
    let app = api(AppState::new(9123));
    let status = |headers: &[(&'static str, &'static str)]| {
        let mut request = Request::builder().uri("/health");
        for &(name, value) in headers {
            request = request.header(name, value);
        }
        let app = app.clone();
        async move {
            app.oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status()
        }
    };
    let local = [
        ("host", "127.0.0.1:9123"),
        ("origin", "http://localhost:9123"),
    ];
    assert_eq!(status(&local).await, StatusCode::OK);
    assert_eq!(
        status(&[("origin", "http://127.0.0.1:5173")]).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&[("host", "127.0.0.1:8080")]).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(&[("origin", "http://localhost:8080")]).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "representative 200,000-feature responsiveness exercise"]
async fn large_workspace_keeps_http_responsive_and_discards_stale_jobs() {
    use std::{
        fmt::Write,
        time::{Duration, Instant},
    };
    let app = api(AppState::default());
    let (_, workspace) = call(&app, "POST", "/workspaces", Value::Null).await;
    let id = workspace["id"].as_str().unwrap();
    let mut dataset = String::from("{\"type\":\"FeatureCollection\",\"features\":[");
    for i in 0..200_000 {
        if i > 0 {
            dataset.push(',');
        }
        write!(&mut dataset,"{{\"type\":\"Feature\",\"id\":{i},\"properties\":{{\"name\":\"point {i}\"}},\"geometry\":{{\"type\":\"Point\",\"coordinates\":[{},{}]}}}}",i%1000,i/1000).unwrap();
    }
    dataset.push_str("]}");
    let start = Instant::now();
    let (status, bytes) = multipart(
        &app,
        &format!("/workspaces/{id}/uploads"),
        "large.geojson",
        dataset.as_bytes(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let imported: Value = serde_json::from_slice(&bytes).unwrap();
    println!(
        "Imported 200,000 features ({} MiB) in {:?}",
        dataset.len() / 1024 / 1024,
        start.elapsed()
    );
    let layer = imported["layers"][0]["id"].as_str().unwrap();
    let mut config = imported["config"].clone();
    config["checks"] = json!([{"rule":"geometry/must-not-be-multipart","layer":layer,"other":null,"enabled":true}]);
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!("/workspaces/{id}/config"),
            config.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", &format!("/workspaces/{id}/runs"), Value::Null)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        call(&app, "POST", &format!("/workspaces/{id}/runs"), Value::Null)
            .await
            .0,
        StatusCode::CONFLICT
    );
    config["tolerance"] = json!(0.1);
    assert_eq!(
        call(&app, "PUT", &format!("/workspaces/{id}/config"), config)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(completed(&app, id).await["job"]["status"], "stale");
    let start = Instant::now();
    call(&app, "POST", &format!("/workspaces/{id}/runs"), Value::Null).await;
    let mut maximum = Duration::ZERO;
    let mut requests = 0;
    loop {
        let request = Instant::now();
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(2),
                call(&app, "GET", "/health", Value::Null)
            )
            .await
            .unwrap()
            .0,
            StatusCode::OK
        );
        maximum = maximum.max(request.elapsed());
        requests += 1;
        let (_, job) = call(&app, "GET", &format!("/workspaces/{id}/job"), Value::Null).await;
        if job["job"]["status"] != "running" {
            assert_eq!(job["job"]["status"], "completed");
            assert_eq!(job["result"]["total"], 0);
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let (_, preview) = call(
        &app,
        "GET",
        &format!("/workspaces/{id}/layers/{layer}/features?bbox=-1,-1,1000,200"),
        Value::Null,
    )
    .await;
    assert_eq!(preview["incomplete"], true);
    assert_eq!(preview["features"].as_array().unwrap().len(), 10_000);
    println!(
        "Validation {:?}; {requests} concurrent health requests; maximum response {:?}",
        start.elapsed(),
        maximum
    );
}

#[cfg(feature = "gdal-system")]
#[tokio::test]
async fn geopackage_layer_picker_and_zipped_shapefile_upload() {
    use gdal::{
        DriverManager,
        vector::{Geometry, LayerAccess, LayerOptions, OGRwkbGeometryType},
    };
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let gpkg = dir.path().join("two.gpkg");
    {
        let mut data = DriverManager::get_driver_by_name("GPKG")
            .unwrap()
            .create_vector_only(&gpkg)
            .unwrap();
        for name in ["first", "second"] {
            let mut layer = data
                .create_layer(LayerOptions {
                    name,
                    ty: OGRwkbGeometryType::wkbPoint,
                    ..Default::default()
                })
                .unwrap();
            layer
                .create_feature(Geometry::from_wkt("POINT (0 0)").unwrap())
                .unwrap();
        }
    }
    let app = api(AppState::default());
    let (_, ws) = call(&app, "POST", "/workspaces", Value::Null).await;
    let id = ws["id"].as_str().unwrap();
    let (status, bytes) = multipart(
        &app,
        &format!("/workspaces/{id}/uploads"),
        "two.gpkg",
        &std::fs::read(gpkg).unwrap(),
    )
    .await;
    let ws: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{ws}");
    assert_eq!(ws["layers"].as_array().unwrap().len(), 2);
    assert!(
        ws["config"]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["selected"] == false)
    );
    let mut config = ws["config"].clone();
    config["layers"][0]["selected"] = json!(true);
    assert_eq!(
        call(&app, "PUT", &format!("/workspaces/{id}/config"), config)
            .await
            .0,
        StatusCode::OK
    );
    let shape = dir.path().join("shape.shp");
    {
        let mut data = DriverManager::get_driver_by_name("ESRI Shapefile")
            .unwrap()
            .create_vector_only(&shape)
            .unwrap();
        let mut layer = data
            .create_layer(LayerOptions {
                ty: OGRwkbGeometryType::wkbPoint,
                ..Default::default()
            })
            .unwrap();
        layer
            .create_feature(Geometry::from_wkt("POINT (1 2)").unwrap())
            .unwrap();
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for extension in ["shp", "shx", "dbf"] {
        zip.start_file(
            format!("folder/shape.{extension}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(&std::fs::read(shape.with_extension(extension)).unwrap())
            .unwrap();
    }
    let zipped = zip.finish().unwrap().into_inner();
    let (status, bytes) = multipart(
        &app,
        &format!("/workspaces/{id}/uploads"),
        "shape.zip",
        &zipped,
    )
    .await;
    let ws: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{ws}");
    assert_eq!(ws["layers"].as_array().unwrap().len(), 3);
    assert!(
        ws["layers"][2]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("missing CRS"))
    );
}

#[tokio::test]
async fn natural_earth_examples_presets_match_direct_core_and_roundtrip_provenance() {
    use std::collections::HashSet;
    use topology_checker_core::Topology;
    use topology_checker_web::{examples, model::geometry};
    let geometry_json = |g: &geo::Geometry| serde_json::to_value(geometry(g)).unwrap();
    let app = api(AppState::default());
    let (status, catalog) = call(&app, "GET", "/examples", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(catalog.as_array().unwrap().len(), 5);
    let (_, current) = call(&app, "POST", "/example", Value::Null).await;
    assert_eq!(
        call(&app, "POST", "/examples/unknown", Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/workspaces/{}", current["id"].as_str().unwrap()),
            Value::Null
        )
        .await
        .1,
        current
    );
    let mut geometry_types = HashSet::new();
    let mut dataset_ids = HashSet::new();
    for example in catalog.as_array().unwrap() {
        let example_id = example["id"].as_str().unwrap();
        let (status, ws) = call(
            &app,
            "POST",
            &format!("/examples/{example_id}"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{ws}");
        assert!(ws["job"].is_null());
        assert!(ws["result"].is_null());
        assert_eq!(ws["config"]["working_crs"], "EPSG:4326");
        assert_eq!(ws["config"]["tolerance"], 1e-8);
        assert_eq!(ws["config"]["view"]["basemap"], false);
        let direct_ws = examples::load(example_id).unwrap();
        let mut direct = Topology::new(direct_ws.config().tolerance).unwrap();
        for (i, layer) in direct_ws.layers().iter().enumerate() {
            assert!(dataset_ids.insert(ws["datasets"][i]["id"].as_str().unwrap().to_owned()));
            assert_eq!(
                ws["layers"][i]["feature_count"],
                example["datasets"][i]["imported_feature_count"]
            );
            assert_eq!(ws["config"]["layers"][i]["selected"], true);
            assert_eq!(
                ws["config"]["layers"][i]["visible"],
                example["datasets"][i]["visible"]
            );
            let bytes = std::fs::read(direct_ws.root().join(&direct_ws.sources()[i].path)).unwrap();
            let original: Value = serde_json::from_slice(&bytes).unwrap();
            let retained = original["features"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|f| {
                    f["geometry"]["coordinates"]
                        .as_array()
                        .is_none_or(|c| !c.is_empty())
                })
                .collect::<Vec<_>>();
            for (ordinal, (_, geometry)) in layer.layer.features().enumerate() {
                let imported = geometry_json(geometry);
                geometry_types.insert(imported["type"].as_str().unwrap().to_owned());
                assert_eq!(
                    imported,
                    geometry_json(
                        &geo::Geometry::try_from(
                            serde_json::from_value::<geojson::Geometry>(
                                retained[ordinal]["geometry"].clone()
                            )
                            .unwrap()
                        )
                        .unwrap()
                    ),
                    "coordinates changed in {example_id}"
                );
                assert_eq!(
                    serde_json::to_value(&layer.attributes[ordinal]).unwrap(),
                    retained[ordinal]["properties"],
                    "attributes changed in {example_id}"
                );
            }
            let id = direct.add_layer(std::sync::Arc::clone(&layer.layer));
            for check in direct_ws
                .config()
                .checks
                .iter()
                .filter(|c| c.layer == layer.id)
            {
                if let Err(error) = direct.add_rule(check.rule, id, None) {
                    assert!(
                        matches!(error, topology_checker_core::Error::InvalidPolygon { .. }),
                        "{error}"
                    );
                }
            }
        }
        let report = direct.validate();
        let id = ws["id"].as_str().unwrap();
        assert_eq!(
            call(&app, "POST", &format!("/workspaces/{id}/runs"), Value::Null)
                .await
                .0,
            StatusCode::ACCEPTED
        );
        let complete = completed(&app, id).await;
        if let Err(error) = &report {
            assert!(matches!(
                error,
                topology_checker_core::Error::InvalidPolygon { .. }
            ));
            assert_eq!(complete["job"]["status"], "failed", "{complete}");
            assert!(
                complete["job"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("polygon")
            );
            assert!(complete["result"].is_null());
        } else {
            let report = report.unwrap();
            assert_eq!(complete["job"]["status"], "completed", "{complete}");
            assert_eq!(complete["result"]["total"], report.violation_count());
            for (i, rule) in report.rules.iter().enumerate() {
                assert_eq!(
                    complete["result"]["rules"][i]["counts"],
                    serde_json::to_value(&rule.counts).unwrap()
                );
                assert_eq!(
                    complete["result"]["rules"][i]["total"],
                    rule.violations.len()
                );
            }
            if example_id == "country-city-atlas" {
                // Multipart detection aggregates collection members into one finding per parent FID.
                assert_eq!(report.violation_count(), 169);
                let all_fids = report.rules[0]
                    .violations
                    .iter()
                    .map(|e| e.origin.unwrap().fid)
                    .collect::<HashSet<_>>();
                assert_eq!(all_fids.len(), report.violation_count());
                let (_, page) = call(
                    &app,
                    "GET",
                    &format!("/workspaces/{id}/results"),
                    Value::Null,
                )
                .await;
                let fids = page["violations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v["origin"]["fid"].as_str().unwrap())
                    .collect::<HashSet<_>>();
                assert_eq!(fids.len(), 100);
            }
        }
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/workspaces/{id}/project"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let (status, reopened) = multipart(
            &app,
            "/projects/open",
            "example.topology-project.zip",
            &bytes,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let reopened: Value = serde_json::from_slice(&reopened).unwrap();
        assert_eq!(ws["datasets"], reopened["datasets"]);
        assert_eq!(ws["config"], reopened["config"]);
        assert_eq!(ws["layers"], reopened["layers"]);
        assert!(reopened["result"].is_null());
    }
    assert_eq!(
        geometry_types,
        [
            "Point",
            "MultiPoint",
            "LineString",
            "MultiLineString",
            "Polygon",
            "MultiPolygon",
            "GeometryCollection"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
}

#[tokio::test]
async fn precision_failure_has_failed_job_status_and_no_results_or_export() {
    let app = api(AppState::default());
    let (_, ws) = call(&app, "POST", "/workspaces", Value::Null).await;
    let id = ws["id"].as_str().unwrap();
    let x = 1f64.next_up();
    let data=json!({"type":"FeatureCollection","features":[
        {"type":"Feature","id":-17,"properties":{},"geometry":{"type":"Polygon","coordinates":[[[1.,0.],[x,0.],[x,1.],[1.,1.],[1.,0.]]]}},
        {"type":"Feature","id":82,"properties":{},"geometry":{"type":"Polygon","coordinates":[[[1.,-1.],[x,-1.],[1.,1.],[1.,-1.]]]}}
    ]}).to_string();
    let (status, bytes) = multipart(
        &app,
        &format!("/workspaces/{id}/uploads"),
        "narrow.geojson",
        data.as_bytes(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let imported: Value = serde_json::from_slice(&bytes).unwrap();
    let layer = imported["layers"][0]["id"].as_str().unwrap();
    let mut config = imported["config"].clone();
    config["tolerance"] = json!(0.);
    config["layers"][0]["selected"] = json!(true);
    config["checks"] =
        json!([{"rule":"polygon/must-not-have-gaps","layer":layer,"other":null,"enabled":true}]);
    let (status, _) = call(&app, "PUT", &format!("/workspaces/{id}/config"), config).await;
    assert_eq!(status, StatusCode::OK);
    call(&app, "POST", &format!("/workspaces/{id}/runs"), Value::Null).await;
    let result = completed(&app, id).await;
    assert_eq!(result["job"]["status"], "failed");
    assert!(
        result["job"]["message"]
            .as_str()
            .unwrap()
            .contains("precision failure")
    );
    assert!(result["result"].is_null());
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/workspaces/{id}/results"),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/workspaces/{id}/export/geojson"),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}
