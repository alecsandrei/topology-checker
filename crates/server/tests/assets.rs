use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use topology_checker_web::assets;
use tower::ServiceExt;

async fn get(path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let app = Router::new().fallback(assets::serve);
    let response = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    (
        parts.status,
        parts.headers,
        body.collect().await.unwrap().to_bytes().to_vec(),
    )
}

fn built_asset() -> String {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/dist/assets");
    let name = std::fs::read_dir(dir)
        .expect("frontend is built")
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .min()
        .expect("assets directory is not empty");
    format!("/assets/{name}")
}

#[tokio::test]
async fn root_and_client_routes_serve_the_application_shell() {
    for path in ["/", "/results/some-route"] {
        let (status, headers, body) = get(path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(
            headers[header::CONTENT_TYPE],
            "text/html; charset=utf-8",
            "{path}"
        );
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache", "{path}");
        assert!(body.starts_with(b"<!doctype html>") || body.starts_with(b"<!DOCTYPE html>"));
    }
}

#[tokio::test]
async fn fingerprinted_assets_are_immutable_and_other_files_are_revalidated() {
    let (status, headers, body) = get(&built_asset()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert!(!body.is_empty());

    let (status, headers, _) = get("/index.html").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn missing_assets_are_not_found_instead_of_the_shell() {
    let (status, _, body) = get("/assets/missing.js").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_empty());
}
