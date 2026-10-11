//! Local topology inspection HTTP API. Heavy work runs on Tokio's blocking pool.
pub mod archive;
pub mod examples;
pub mod export;
pub mod model;
pub mod workspace;

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use model::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{io::AsyncWriteExt, sync::Semaphore};
use uuid::Uuid;
use workspace::{Job, SharedWorkspace, Workspace};

#[derive(Debug, Clone)]
pub struct AppState {
    workspaces: Arc<Mutex<HashMap<String, SharedWorkspace>>>,
    validation: Arc<Semaphore>,
}
impl Default for AppState {
    fn default() -> Self {
        Self {
            workspaces: Arc::new(Mutex::new(HashMap::new())),
            validation: Arc::new(Semaphore::new(1)),
        }
    }
}
#[derive(Debug)]
pub struct ApiError(StatusCode, String);
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self(StatusCode::BAD_REQUEST, format!("{e:#}"))
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
type ApiResult<T> = std::result::Result<T, ApiError>;
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("background task failed: {e}"),
            )
        })?
        .map_err(Into::into)
}
impl AppState {
    fn workspace(&self, id: &str) -> ApiResult<SharedWorkspace> {
        self.workspaces
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "workspace not found".into()))
    }
    fn insert(&self, ws: Workspace) -> Value {
        let value = ws.json();
        self.workspaces
            .lock()
            .unwrap()
            .insert(ws.id.clone(), Arc::new(Mutex::new(ws)));
        value
    }
}
pub fn api(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/rules", get(|| async { Json(catalog()) }))
        .route("/crs", get(resolve_crs))
        .route("/workspaces", post(create))
        .route("/example", post(example))
        .route("/examples", get(|| async { Json(examples::catalog()) }))
        .route("/examples/{id}", post(real_example))
        .route("/projects/open", post(open_project))
        .route(
            "/workspaces/{id}",
            get(workspace_info).delete(remove_workspace),
        )
        .route("/workspaces/{id}/uploads", post(upload))
        .route("/workspaces/{id}/config", axum::routing::put(configure))
        .route("/workspaces/{id}/layers/{layer}/features", get(viewport))
        .route(
            "/workspaces/{id}/layers/{layer}/features/{fid}",
            get(feature),
        )
        .route("/workspaces/{id}/runs", post(run))
        .route("/workspaces/{id}/job", get(job))
        .route("/workspaces/{id}/results", get(results))
        .route("/workspaces/{id}/violations/{index}", get(violation))
        .route("/workspaces/{id}/export/{format}", get(download_report))
        .route("/workspaces/{id}/project", get(download_project))
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"API endpoint not found"})),
            )
        })
        .layer(DefaultBodyLimit::max(
            archive::UPLOAD_LIMIT as usize + 1024 * 1024,
        ))
        .layer(middleware::from_fn(local_request))
        .with_state(state)
}
async fn local_request(
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let host_ok = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_none_or(|h| matches!(h, "127.0.0.1:8080" | "localhost:8080"));
    let origin_ok = headers
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        .is_none_or(|h| {
            matches!(
                h,
                "http://127.0.0.1:8080"
                    | "http://localhost:8080"
                    | "http://127.0.0.1:5173"
                    | "http://localhost:5173"
            )
        });
    if !host_ok || !origin_ok {
        return ApiError(
            StatusCode::FORBIDDEN,
            "use the local application origin".into(),
        )
        .into_response();
    }
    next.run(request).await
}
async fn create(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    blocking(move || Ok(Json(state.insert(Workspace::new()?)))).await
}
async fn workspace_info(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let ws = state.workspace(&id)?;
    blocking(move || Ok(Json(ws.lock().unwrap().json()))).await
}
async fn remove_workspace(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    state.workspaces.lock().unwrap().remove(&id);
    StatusCode::NO_CONTENT
}
#[derive(Deserialize)]
struct CrsQuery {
    definition: String,
}
async fn resolve_crs(Query(q): Query<CrsQuery>) -> ApiResult<Json<Value>> {
    blocking(move || Ok(Json(crs_info(&q.definition)?))).await
}
async fn incoming(mut multipart: Multipart) -> ApiResult<(tempfile::TempDir, String, PathBuf)> {
    let directory = tempfile::tempdir().map_err(anyhow::Error::from)?;
    let mut field = multipart
        .next_field()
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .context("missing upload")?;
    let filename = field.file_name().context("missing filename")?.to_string();
    let relative = archive::safe_path(&filename)?;
    if relative.components().count() != 1 {
        return Err(anyhow::anyhow!("upload filename must be a basename").into());
    }
    let path = directory.path().join(&relative);
    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(anyhow::Error::from)?;
    let mut size = 0_u64;
    while let Some(chunk) = field.chunk().await.map_err(|e| anyhow::anyhow!(e))? {
        size += chunk.len() as u64;
        if size > archive::UPLOAD_LIMIT {
            return Err(ApiError(
                StatusCode::PAYLOAD_TOO_LARGE,
                "upload exceeds 512 MiB".into(),
            ));
        }
        file.write_all(&chunk).await.map_err(anyhow::Error::from)?;
    }
    file.flush().await.map_err(anyhow::Error::from)?;
    drop(field);
    if multipart
        .next_field()
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .is_some()
    {
        return Err(anyhow::anyhow!("upload one dataset at a time").into());
    }
    Ok((directory, filename, path))
}
async fn upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    multipart: Multipart,
) -> ApiResult<Json<Value>> {
    let ws = state.workspace(&id)?;
    let (temporary, filename, path) = incoming(multipart).await?;
    blocking(move || {
        let (root, version, config, existing) = {
            let w = ws.lock().unwrap();
            (
                w.storage.path().to_path_buf(),
                w.version,
                w.config.clone(),
                w.layers.clone(),
            )
        };
        let pending = workspace::PendingImport::copy(&root, filename, &path)?;
        let layers = workspace::import(&pending.source, &root)?;
        // Build prospective indexes outside the workspace lock.
        let addition = workspace::Addition::prepare(config, existing, layers)?;
        let mut w = ws.lock().unwrap();
        if w.version != version {
            bail!("workspace changed during import; retry upload");
        }
        addition.commit(&mut w, pending.commit());
        drop(temporary);
        Ok(Json(w.json()))
    })
    .await
}
async fn configure(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut config): Json<Config>,
) -> ApiResult<Json<Value>> {
    let ws = state.workspace(&id)?;
    blocking(move || {
        let (old, sources, version, previous) = {
            let w = ws.lock().unwrap();
            (w.config.clone(), w.layers.clone(), w.version, w.prepared.clone())
        };
        normalize_config(&mut config, &old, &sources)?;
        let signature = |c: &Config| {
            json!({
                "crs": c.working_crs,
                "tolerance": c.tolerance,
                "checks": c.checks,
                "layers": c.layers.iter().map(|l| json!([l.id, l.selected, l.source_crs])).collect::<Vec<_>>()
            })
        };
        let changed = signature(&config) != signature(&old);
        let geometry_changed = config.working_crs != old.working_crs
            || config.layers.iter().any(|l| {
                !old.layers.iter().any(|o| {
                    o.id == l.id && o.selected == l.selected && o.source_crs == l.source_crs
                })
            });
        let prepared = if geometry_changed { prepare(&config, &sources)? } else { previous };
        let mut w = ws.lock().unwrap();
        if version != w.version { bail!("workspace changed during configuration; retry"); }
        w.config = config;
        w.prepared = prepared;
        w.version += 1;
        if changed { w.invalidate(); }
        Ok(Json(w.json()))
    }).await
}
#[derive(Deserialize)]
struct ViewQuery {
    bbox: String,
    #[serde(default)]
    display: bool,
}
fn bounds(s: &str) -> Result<[f64; 4]> {
    let v = s
        .split(',')
        .map(str::parse)
        .collect::<std::result::Result<Vec<f64>, _>>()?;
    let b: [f64; 4] = v
        .try_into()
        .map_err(|_| anyhow::anyhow!("bbox needs four coordinates"))?;
    if b.iter().any(|v| !v.is_finite()) || b[0] > b[2] || b[1] > b[3] {
        bail!("invalid viewport bounds");
    }
    Ok(b)
}
fn prepared(ws: &SharedWorkspace, id: &str) -> Result<(Arc<IndexedLayer>, Option<String>)> {
    let w = ws.lock().unwrap();
    Ok((
        Arc::clone(
            w.prepared
                .iter()
                .find(|l| l.source.id == id)
                .context("layer is not selected")?,
        ),
        w.config.working_crs.clone(),
    ))
}
async fn viewport(
    State(state): State<AppState>,
    Path((id, layer)): Path<(String, String)>,
    Query(q): Query<ViewQuery>,
) -> ApiResult<Json<Value>> {
    let b = bounds(&q.bbox)?;
    let (layer, crs) = prepared(&state.workspace(&id)?, &layer)?;
    blocking(move || Ok(Json(layer.viewport(b, crs.as_deref(), q.display)?))).await
}
#[derive(Deserialize, Default)]
struct DisplayQuery {
    #[serde(default)]
    display: bool,
}
async fn feature(
    State(state): State<AppState>,
    Path((id, layer, fid)): Path<(String, String, String)>,
    Query(q): Query<DisplayQuery>,
) -> ApiResult<Json<Value>> {
    let fid = fid.parse::<i64>().map_err(anyhow::Error::from)?;
    let (layer, working) = prepared(&state.workspace(&id)?, &layer)?;
    blocking(move || {
        let mut f = layer.feature(fid).context("feature not found")?;
        if q.display {
            let value: geojson::Geometry = serde_json::from_value(f["geometry"].clone())?;
            let geometry: geo::Geometry = value.try_into()?;
            let from = topology_checker_io::crs::parse(
                working.as_deref().context("working CRS is unknown")?,
            )?;
            let to = topology_checker_io::crs::parse("EPSG:3857")?;
            f["geometry"] = geometry_json(&topology_checker_io::crs::transform(
                &geometry,
                &gdal::spatial_ref::CoordTransform::new(&from, &to)?,
            )?);
        }
        Ok(Json(f))
    })
    .await
}
async fn run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ws = state.workspace(&id)?;
    let permit = Arc::clone(&state.validation)
        .try_acquire_owned()
        .map_err(|_| {
            ApiError(
                StatusCode::CONFLICT,
                "a validation job is already running".into(),
            )
        })?;
    let (revision, config, layers, id) = {
        let mut w = ws.lock().unwrap();
        let id = Uuid::new_v4().to_string();
        let config = w.config.clone();
        w.result = None;
        w.job = Some(Job {
            id: id.clone(),
            revision: w.revision,
            status: "running".into(),
            message: None,
            config: config.clone(),
        });
        (w.revision, config, w.prepared.clone(), id)
    };
    let response = json!({"id":id,"revision":revision,"status":"running"});
    tokio::spawn(async move {
        let result =
            tokio::task::spawn_blocking(move || ValidationResult::run(revision, config, layers))
                .await;
        let mut w = ws.lock().unwrap();
        let stale = w.revision != revision;
        let (status, message) = match result {
            Ok(Ok(r)) if !stale => {
                w.result = Some(Arc::new(r));
                ("completed", None)
            }
            Ok(Ok(_)) => ("stale", Some("workspace changed; run checks again".into())),
            Ok(Err(e)) => ("failed", Some(format!("{e:#}"))),
            Err(e) => ("failed", Some(format!("validation task failed: {e}"))),
        };
        if let Some(job) = &mut w.job {
            job.status = status.into();
            job.message = message;
        }
        drop(permit);
    });
    Ok((StatusCode::ACCEPTED, Json(response)))
}
async fn job(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let ws = state.workspace(&id)?;
    let w = ws.lock().unwrap();
    Ok(Json(
        json!({"job":w.job,"result":w.result.as_ref().map(|r|r.summary())}),
    ))
}
fn result(ws: &SharedWorkspace) -> ApiResult<Arc<ValidationResult>> {
    ws.lock().unwrap().result.clone().ok_or_else(|| {
        ApiError(
            StatusCode::CONFLICT,
            "run checks to generate current results".into(),
        )
    })
}
#[derive(Deserialize)]
struct PageQuery {
    #[serde(default)]
    page: usize,
}
async fn results(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let r = result(&state.workspace(&id)?)?;
    blocking(move || Ok(Json(r.page(q.page)?))).await
}
async fn violation(
    State(state): State<AppState>,
    Path((id, index)): Path<(String, usize)>,
    Query(q): Query<DisplayQuery>,
) -> ApiResult<Json<Value>> {
    let r = result(&state.workspace(&id)?)?;
    blocking(move || Ok(Json(r.violation(index, q.display)?))).await
}
fn file_download(
    directory: tempfile::TempDir,
    path: &std::path::Path,
    name: &str,
    content_type: &str,
) -> Result<Response> {
    let file = tokio::fs::File::from_std(std::fs::File::open(path)?);
    // The response owns its temporary output until completion or disconnect.
    let stream = tokio_util::io::ReaderStream::new(file).map(move |chunk| {
        let _keep_alive = &directory;
        chunk
    });
    Ok((
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}
async fn download_report(
    State(state): State<AppState>,
    Path((id, format)): Path<(String, String)>,
) -> ApiResult<Response> {
    let r = result(&state.workspace(&id)?)?;
    blocking(move || {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join(format!("report.{format}"));
        let (name, content_type) = match format.as_str() {
            "json" => {
                export::write_json(&r, std::io::BufWriter::new(std::fs::File::create(&path)?))?;
                ("topology-report.json", "application/json")
            }
            "geojson" => {
                export::write_geojson(&r, std::io::BufWriter::new(std::fs::File::create(&path)?))?;
                ("violations.geojson", "application/geo+json")
            }
            "gpkg" => {
                export::geopackage(&r, &path)?;
                ("violations.gpkg", "application/geopackage+sqlite3")
            }
            _ => bail!("unknown export format"),
        };
        file_download(directory, &path, name, content_type)
    })
    .await
}
async fn download_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let ws = state.workspace(&id)?;
    blocking(move || {
        let snapshot = workspace::project_parts(&ws.lock().unwrap())?;
        workspace::check_project_size(&snapshot)?;
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("project.zip");
        archive::pack(&path, &snapshot.manifest, &snapshot.files)?;
        let response = file_download(
            directory,
            &path,
            "workspace.topology-project.zip",
            "application/zip",
        )?;
        drop(ws);
        Ok(response)
    })
    .await
}
async fn open_project(
    State(state): State<AppState>,
    multipart: Multipart,
) -> ApiResult<Json<Value>> {
    let (temporary, _, path) = incoming(multipart).await?;
    blocking(move || {
        let ws = workspace::reopen(&path)?;
        drop(temporary);
        Ok(Json(state.insert(ws)))
    })
    .await
}
async fn example(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    blocking(move || {
        let mut ws = Workspace::new()?;
        let source = DatasetSource {
            provenance: None,
            id: "example".into(),
            filename: "streets.geojson".into(),
            path: PathBuf::from("sources/example/streets.geojson"),
        };
        let path = ws.storage.path().join(&source.path);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, include_str!("../example.geojson"))?;
        let layers = workspace::import(&source, ws.storage.path())?;
        workspace::append(&mut ws, source, layers)?;
        ws.config.working_crs = Some("EPSG:28992".into());
        ws.config.tolerance = 0.001;
        ws.config.checks = vec![
            Check {
                rule: topology_checker_core::Rule::LineMustNotHaveDangles,
                layer: ws.layers[0].id.clone(),
                other: None,
                enabled: true,
            },
            Check {
                rule: topology_checker_core::Rule::LineMustNotOverlap,
                layer: ws.layers[0].id.clone(),
                other: None,
                enabled: true,
            },
        ];
        ws.prepared = prepare(&ws.config, &ws.layers)?;
        Ok(Json(state.insert(ws)))
    })
    .await
}

async fn real_example(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !examples::catalog()
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["id"] == id)
    {
        return Err(ApiError(StatusCode::NOT_FOUND, "example not found".into()));
    }
    blocking(move || Ok(Json(state.insert(examples::load(&id)?)))).await
}
