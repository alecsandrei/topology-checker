//! Local topology inspection HTTP API. Heavy work runs on Tokio's blocking pool.
pub mod archive;
pub mod assets;
pub mod dto;
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
use dto::{
    CrsInfo, ErrorBody, Example, JobPoll, JobStatus, MapFeature, Page, RuleInfo, RunAccepted,
    Viewport, Violation, WorkspaceView,
};
use futures_util::StreamExt;
use model::*;
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{io::AsyncWriteExt, sync::Semaphore};
use workspace::{SharedWorkspace, Workspace};

/// Host and Origin values accepted from the local browser, guarding against DNS rebinding.
#[derive(Debug)]
struct LocalOrigins {
    hosts: [String; 2],
    origins: [String; 4],
}
impl LocalOrigins {
    fn new(port: u16) -> Self {
        Self {
            hosts: [format!("127.0.0.1:{port}"), format!("localhost:{port}")],
            // The Vite development server proxies API requests from port 5173.
            origins: [
                format!("http://127.0.0.1:{port}"),
                format!("http://localhost:{port}"),
                "http://127.0.0.1:5173".into(),
                "http://localhost:5173".into(),
            ],
        }
    }
}

/// Workspaces unused for this long are discarded with their temporary files.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(4 * 60 * 60);

#[derive(Debug)]
struct Tracked {
    workspace: SharedWorkspace,
    last_used: Instant,
}
#[derive(Debug, Clone)]
pub struct AppState {
    workspaces: Arc<Mutex<HashMap<String, Tracked>>>,
    validation: Arc<Semaphore>,
    local: Arc<LocalOrigins>,
}
impl Default for AppState {
    fn default() -> Self {
        Self::new(8080)
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
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
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
    /// Create empty state for a server listening on `port` on the loopback interface.
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self {
            workspaces: Arc::new(Mutex::new(HashMap::new())),
            validation: Arc::new(Semaphore::new(1)),
            local: Arc::new(LocalOrigins::new(port)),
        }
    }
    fn workspace(&self, id: &str) -> ApiResult<SharedWorkspace> {
        let mut workspaces = self.workspaces.lock().unwrap();
        let tracked = workspaces
            .get_mut(id)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "workspace not found".into()))?;
        tracked.last_used = Instant::now();
        Ok(Arc::clone(&tracked.workspace))
    }
    fn insert(&self, ws: Workspace) -> WorkspaceView {
        let value = ws.to_view();
        self.workspaces.lock().unwrap().insert(
            ws.id().to_owned(),
            Tracked {
                workspace: Arc::new(Mutex::new(ws)),
                last_used: Instant::now(),
            },
        );
        value
    }
    /// Discard workspaces unused for at least `idle`, except those that are busy or
    /// running a validation job, and return how many were discarded.
    pub fn evict_idle(&self, idle: Duration) -> usize {
        let now = Instant::now();
        let mut workspaces = self.workspaces.lock().unwrap();
        let before = workspaces.len();
        workspaces.retain(|_, tracked| {
            now.duration_since(tracked.last_used) < idle
                || tracked
                    .workspace
                    .try_lock()
                    .map_or(true, |ws| ws.job_running())
        });
        before - workspaces.len()
    }
}
pub fn api(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/rules", get(|| async { Json::<Vec<RuleInfo>>(catalog()) }))
        .route(
            "/formats",
            get(|| async { Json(topology_checker_io::registry::formats()) }),
        )
        .route("/crs", get(resolve_crs))
        .route("/workspaces", post(create))
        .route("/example", post(example))
        .route(
            "/examples",
            get(|| async { Json::<&[Example]>(examples::catalog()) }),
        )
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
                Json(ErrorBody {
                    error: "API endpoint not found".into(),
                }),
            )
        })
        .layer(DefaultBodyLimit::max(
            archive::UPLOAD_LIMIT as usize + 1024 * 1024,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), local_request))
        .with_state(state)
}
async fn local_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let allowed = |name, values: &[String]| {
        headers
            .get(name)
            .and_then(|h| h.to_str().ok())
            .is_none_or(|h| values.iter().any(|v| v == h))
    };
    let host_ok = allowed(header::HOST, &state.local.hosts);
    let origin_ok = allowed(header::ORIGIN, &state.local.origins);
    if !host_ok || !origin_ok {
        return ApiError(
            StatusCode::FORBIDDEN,
            "use the local application origin".into(),
        )
        .into_response();
    }
    next.run(request).await
}
async fn create(State(state): State<AppState>) -> ApiResult<Json<WorkspaceView>> {
    blocking(move || Ok(Json(state.insert(Workspace::new()?)))).await
}
async fn workspace_info(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkspaceView>> {
    let ws = state.workspace(&id)?;
    blocking(move || Ok(Json(ws.lock().unwrap().to_view()))).await
}
async fn remove_workspace(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    state.workspaces.lock().unwrap().remove(&id);
    StatusCode::NO_CONTENT
}
#[derive(Deserialize)]
struct CrsQuery {
    definition: String,
}
async fn resolve_crs(Query(q): Query<CrsQuery>) -> ApiResult<Json<CrsInfo>> {
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
) -> ApiResult<Json<WorkspaceView>> {
    let ws = state.workspace(&id)?;
    let (temporary, filename, path) = incoming(multipart).await?;
    blocking(move || {
        let snapshot = ws.lock().unwrap().snapshot();
        // Copy, read and index outside the workspace lock.
        let pending = workspace::PendingImport::copy(&snapshot.root, filename, &path)?;
        let layers = workspace::import(&pending.source, &snapshot.root)?;
        let addition =
            workspace::Addition::prepare(snapshot.config.clone(), snapshot.layers.clone(), layers)?;
        let mut w = ws.lock().unwrap();
        w.commit_import(&snapshot, addition, pending)?;
        drop(temporary);
        Ok(Json(w.to_view()))
    })
    .await
}
async fn configure(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut config): Json<Config>,
) -> ApiResult<Json<WorkspaceView>> {
    let ws = state.workspace(&id)?;
    blocking(move || {
        let snapshot = ws.lock().unwrap().snapshot();
        normalize_config(&mut config, &snapshot.config, &snapshot.layers)?;
        // Reprojection and indexing happen outside the lock, and only when needed.
        let prepared = if config.geometry_differs(&snapshot.config) {
            prepare(&config, &snapshot.layers)?
        } else {
            Arc::clone(&snapshot.prepared)
        };
        let mut w = ws.lock().unwrap();
        w.commit_config(&snapshot, config, prepared)?;
        Ok(Json(w.to_view()))
    })
    .await
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
        Arc::clone(w.prepared().layer(id).context("layer is not selected")?),
        w.config().working_crs.clone(),
    ))
}
async fn viewport(
    State(state): State<AppState>,
    Path((id, layer)): Path<(String, String)>,
    Query(q): Query<ViewQuery>,
) -> ApiResult<Json<Viewport>> {
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
) -> ApiResult<Json<MapFeature>> {
    let fid = fid.parse::<i64>().map_err(anyhow::Error::from)?;
    let (layer, working) = prepared(&state.workspace(&id)?, &layer)?;
    blocking(move || {
        Ok(Json(
            layer
                .feature(fid, working.as_deref(), q.display)?
                .context("feature not found")?,
        ))
    })
    .await
}
async fn run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<RunAccepted>)> {
    let ws = state.workspace(&id)?;
    let permit = Arc::clone(&state.validation)
        .try_acquire_owned()
        .map_err(|_| {
            ApiError(
                StatusCode::CONFLICT,
                "a validation job is already running".into(),
            )
        })?;
    let ticket = ws.lock().unwrap().start_job();
    let response = RunAccepted {
        id: ticket.id.clone(),
        revision: ticket.revision,
        status: JobStatus::Running,
    };
    tokio::spawn(async move {
        let (revision, config, prepared) = (
            ticket.revision,
            ticket.config.clone(),
            Arc::clone(&ticket.prepared),
        );
        let outcome =
            tokio::task::spawn_blocking(move || ValidationResult::run(revision, config, &prepared))
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("validation task failed: {e}")));
        ws.lock().unwrap().finish_job(&ticket, outcome);
        drop(permit);
    });
    Ok((StatusCode::ACCEPTED, Json(response)))
}
async fn job(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<JobPoll>> {
    Ok(Json(state.workspace(&id)?.lock().unwrap().poll()))
}
fn result(ws: &SharedWorkspace) -> ApiResult<Arc<ValidationResult>> {
    ws.lock().unwrap().result().ok_or_else(|| {
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
) -> ApiResult<Json<Page>> {
    let r = result(&state.workspace(&id)?)?;
    blocking(move || Ok(Json(r.page(q.page)?))).await
}
async fn violation(
    State(state): State<AppState>,
    Path((id, index)): Path<(String, usize)>,
    Query(q): Query<DisplayQuery>,
) -> ApiResult<Json<Violation>> {
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
                ("topology-report.json".to_string(), "application/json")
            }
            "geojson" => {
                export::write_geojson(&r, std::io::BufWriter::new(std::fs::File::create(&path)?))?;
                ("violations.geojson".to_string(), "application/geo+json")
            }
            "gpkg" => {
                export::dataset(&r, &path)?;
                (
                    "violations.gpkg".to_string(),
                    "application/geopackage+sqlite3",
                )
            }
            _ => {
                let f = topology_checker_io::registry::for_path(&path)
                    .filter(|f| f.native && f.web_export && f.write)
                    .context("unknown export format")?;
                export::dataset(&r, &path)?;
                (
                    format!("violations.{format}"),
                    match f.name.as_str() {
                        "KML" if format == "kmz" => "application/vnd.google-earth.kmz",
                        "KML" => "application/vnd.google-earth.kml+xml",
                        "GeoParquet" => "application/vnd.apache.parquet",
                        "GPX" => "application/gpx+xml",
                        _ => "application/octet-stream",
                    },
                )
            }
        };
        file_download(directory, &path, &name, content_type)
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
) -> ApiResult<Json<WorkspaceView>> {
    let (temporary, _, path) = incoming(multipart).await?;
    blocking(move || {
        let ws = workspace::reopen(&path)?;
        drop(temporary);
        Ok(Json(state.insert(ws)))
    })
    .await
}
async fn example(State(state): State<AppState>) -> ApiResult<Json<WorkspaceView>> {
    blocking(move || {
        let mut ws = Workspace::new()?;
        let source = DatasetSource {
            provenance: None,
            id: "example".into(),
            filename: "streets.geojson".into(),
            path: PathBuf::from("sources/example/streets.geojson"),
        };
        let path = ws.root().join(&source.path);
        std::fs::create_dir_all(path.parent().context("source directory")?)?;
        std::fs::write(&path, include_str!("../example.geojson"))?;
        let layers = workspace::import(&source, ws.root())?;
        workspace::append(&mut ws, source, layers)?;
        let streets = ws.layers()[0].id.clone();
        let mut config = ws.config().clone();
        config.working_crs = Some("EPSG:28992".into());
        config.tolerance = 0.001;
        config.checks = vec![
            Check {
                rule: topology_checker_core::Rule::LineMustNotHaveDangles,
                layer: streets.clone(),
                other: None,
                enabled: true,
            },
            Check {
                rule: topology_checker_core::Rule::LineMustNotOverlap,
                layer: streets,
                other: None,
                enabled: true,
            },
        ];
        ws.initialize(config)?;
        Ok(Json(state.insert(ws)))
    })
    .await
}

async fn real_example(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkspaceView>> {
    if !examples::catalog().iter().any(|e| e.id == id) {
        return Err(ApiError(StatusCode::NOT_FOUND, "example not found".into()));
    }
    blocking(move || Ok(Json(state.insert(examples::load(&id)?)))).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_workspaces_are_evicted_unless_running_or_busy() {
        let state = AppState::default();
        let ids = (0..3)
            .map(|_| state.insert(Workspace::new().unwrap()).id)
            .collect::<Vec<_>>();
        let running = state.workspace(&ids[0]).unwrap();
        let _ticket = running.lock().unwrap().start_job();
        let busy = state.workspace(&ids[1]).unwrap();
        let _guard = busy.lock().unwrap();
        assert_eq!(state.evict_idle(IDLE_TIMEOUT), 0, "recently used");
        assert_eq!(state.evict_idle(Duration::ZERO), 1);
        assert!(state.workspace(&ids[0]).is_ok());
        assert!(state.workspace(&ids[1]).is_ok());
        assert!(state.workspace(&ids[2]).is_err());
    }
}
