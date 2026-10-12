use crate::{
    archive,
    dto::{CrsInfo, DatasetView, Job, JobPoll, JobStatus, WorkspaceView},
    model::*,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use topology_checker_io::read;
use uuid::Uuid;

/// Imported datasets, their configuration and the latest validation job.
///
/// Edits follow one pattern: take a [`Snapshot`] under the lock, do slow work such as
/// reading or reprojecting outside it, then commit with the snapshot. A commit fails
/// when another edit landed in between, so a client retries rather than losing work.
#[derive(Debug)]
pub struct Workspace {
    id: String,
    storage: Arc<TempDir>,
    sources: Vec<DatasetSource>,
    layers: Vec<Arc<SourceLayer>>,
    prepared: Arc<Prepared>,
    config: Config,
    /// The parsed working CRS, kept with `config` so responses do not parse it again.
    crs_info: Option<CrsInfo>,
    /// Results generation shared with clients. Any edit that can change validation
    /// results increments it and discards the current result.
    revision: u64,
    /// Concurrency token incremented by every committed edit.
    edit_version: u64,
    job: Option<Job>,
    result: Option<Arc<ValidationResult>>,
}
/// Workspace state captured under the lock for preparing an edit outside it.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub root: PathBuf,
    pub config: Config,
    pub layers: Vec<Arc<SourceLayer>>,
    pub prepared: Arc<Prepared>,
    edit_version: u64,
}
/// The inputs of a started validation job.
#[derive(Debug)]
pub struct JobTicket {
    pub id: String,
    pub revision: u64,
    pub config: Config,
    pub prepared: Arc<Prepared>,
    pub storage: Arc<TempDir>,
}
impl Workspace {
    /// Create a temporary local workspace.
    ///
    /// # Errors
    /// Returns an error when temporary storage cannot be created.
    pub fn new() -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            storage: Arc::new(tempfile::tempdir()?),
            sources: vec![],
            layers: vec![],
            prepared: Arc::default(),
            config: Config::default(),
            crs_info: None,
            revision: 0,
            edit_version: 0,
            job: None,
            result: None,
        })
    }
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The temporary directory holding original and extracted sources.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.storage.path()
    }
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }
    #[must_use]
    pub fn sources(&self) -> &[DatasetSource] {
        &self.sources
    }
    #[must_use]
    pub fn layers(&self) -> &[Arc<SourceLayer>] {
        &self.layers
    }
    #[must_use]
    pub fn prepared(&self) -> &Arc<Prepared> {
        &self.prepared
    }
    /// The result of the latest completed job for the current revision.
    #[must_use]
    pub fn result(&self) -> Option<Arc<ValidationResult>> {
        self.result.clone()
    }
    #[must_use]
    pub fn job_running(&self) -> bool {
        self.job
            .as_ref()
            .is_some_and(|j| j.status == JobStatus::Running)
    }
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            root: self.root().to_path_buf(),
            config: self.config.clone(),
            layers: self.layers.clone(),
            prepared: Arc::clone(&self.prepared),
            edit_version: self.edit_version,
        }
    }
    fn set_config(&mut self, config: Config) {
        self.crs_info = config.working_crs.as_deref().and_then(|c| crs_info(c).ok());
        self.config = config;
    }
    /// Set the configuration of a workspace that is not shared yet and prepare its layers.
    ///
    /// # Errors
    /// Returns an error if the working geometry cannot be prepared.
    pub(crate) fn initialize(&mut self, config: Config) -> Result<()> {
        self.prepared = prepare(&config, &self.layers)?;
        self.set_config(config);
        Ok(())
    }
    /// Apply a normalized configuration and the layers prepared for it from `snapshot`.
    ///
    /// # Errors
    /// Returns an error if another edit was committed after the snapshot.
    pub fn commit_config(
        &mut self,
        snapshot: &Snapshot,
        config: Config,
        prepared: Arc<Prepared>,
    ) -> Result<()> {
        if snapshot.edit_version != self.edit_version {
            bail!("workspace changed during configuration; retry");
        }
        let invalidates = config.validation_differs(&self.config);
        self.set_config(config);
        self.prepared = prepared;
        self.edit_version += 1;
        if invalidates {
            self.invalidate();
        }
        Ok(())
    }
    /// Accept an import prepared from `snapshot`, keeping its copied files.
    ///
    /// # Errors
    /// Returns an error if another edit was committed after the snapshot. The pending
    /// import then removes its files when dropped.
    pub(crate) fn commit_import(
        &mut self,
        snapshot: &Snapshot,
        addition: Addition,
        pending: PendingImport,
    ) -> Result<()> {
        if snapshot.edit_version != self.edit_version {
            bail!("workspace changed during import; retry upload");
        }
        addition.apply(self, pending.commit());
        Ok(())
    }
    /// Record a running job for the current configuration and discard the old result.
    pub fn start_job(&mut self) -> JobTicket {
        let ticket = JobTicket {
            id: Uuid::new_v4().to_string(),
            revision: self.revision,
            config: self.config.clone(),
            prepared: Arc::clone(&self.prepared),
            storage: Arc::clone(&self.storage),
        };
        self.result = None;
        self.job = Some(Job {
            id: ticket.id.clone(),
            revision: ticket.revision,
            status: JobStatus::Running,
            message: None,
            config: ticket.config.clone(),
        });
        ticket
    }
    /// Store a job's outcome. A result for an outdated revision is reported as stale.
    pub fn finish_job(&mut self, ticket: &JobTicket, outcome: Result<ValidationResult>) {
        // A previous run at the same revision must not overwrite a newer job.
        if self.job.as_ref().is_none_or(|job| job.id != ticket.id) {
            return;
        }
        let stale = self.revision != ticket.revision;
        let (status, message) = match outcome {
            Ok(result) if !stale => {
                self.result = Some(Arc::new(result));
                (JobStatus::Completed, None)
            }
            Ok(_) => (
                JobStatus::Stale,
                Some("workspace changed; run checks again".into()),
            ),
            Err(e) => (JobStatus::Failed, Some(format!("{e:#}"))),
        };
        if let Some(job) = self.job.as_mut().filter(|j| j.id == ticket.id) {
            job.status = status;
            job.message = message;
        }
    }
    #[must_use]
    pub fn poll(&self) -> JobPoll {
        JobPoll {
            job: self.job.clone(),
            result: self.result.as_ref().map(|r| r.summary()),
        }
    }
    #[must_use]
    pub fn to_view(&self) -> WorkspaceView {
        WorkspaceView {
            id: self.id.clone(),
            revision: self.revision,
            config: self.config.clone(),
            working_crs_info: self.crs_info.clone(),
            datasets: self
                .sources
                .iter()
                .map(|s| DatasetView {
                    id: s.id.clone(),
                    filename: s.filename.clone(),
                    provenance: s.provenance.clone(),
                })
                .collect(),
            layers: layer_metadata(&self.layers, &self.prepared),
            job: self.job.clone(),
            result: self.result.as_ref().map(|r| r.summary()),
        }
    }
    fn invalidate(&mut self) {
        self.revision += 1;
        self.result = None;
    }
}
pub type SharedWorkspace = Arc<Mutex<Workspace>>;

/// Own copied and expanded upload files until the workspace accepts the import.
#[derive(Debug)]
pub(crate) struct PendingImport {
    pub source: DatasetSource,
    root: PathBuf,
    committed: bool,
}
impl PendingImport {
    pub fn copy(root: &Path, filename: String, path: &Path) -> Result<Self> {
        let filename_path = archive::safe_path(&filename)?;
        if filename_path.components().count() != 1 {
            bail!("upload filename must be a basename");
        }
        let id = Uuid::new_v4().to_string();
        let pending = Self {
            source: DatasetSource {
                path: PathBuf::from("sources").join(&id).join(&filename),
                id,
                filename,
                provenance: None,
            },
            root: root.to_path_buf(),
            committed: false,
        };
        let destination = root.join(&pending.source.path);
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::copy(path, destination)?;
        Ok(pending)
    }
    pub fn commit(mut self) -> DatasetSource {
        self.committed = true;
        self.source.clone()
    }
}
impl Drop for PendingImport {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(self.root.join("sources").join(&self.source.id));
            let _ = fs::remove_dir_all(self.root.join(format!("extracted-{}", self.source.id)));
        }
    }
}

/// Layers and configuration prepared outside the lock for an import.
#[derive(Debug)]
pub(crate) struct Addition {
    config: Config,
    layers: Vec<Arc<SourceLayer>>,
    prepared: Arc<Prepared>,
}
impl Addition {
    pub fn prepare(
        mut config: Config,
        mut existing: Vec<Arc<SourceLayer>>,
        imported: Vec<Arc<SourceLayer>>,
    ) -> Result<Self> {
        let multiple = imported.len() > 1;
        let colors = ["#28786f", "#577ec0", "#b77535", "#9867a7", "#d46b58"];
        for (i, layer) in imported.iter().enumerate() {
            config.layers.push(LayerSettings {
                id: layer.id.clone(),
                selected: !multiple && config.working_crs.is_none(),
                visible: true,
                color: colors[(existing.len() + i) % colors.len()].into(),
                source_crs: None,
            });
        }
        existing.extend(imported);
        let prepared = prepare(&config, &existing)?;
        Ok(Self {
            config,
            layers: existing,
            prepared,
        })
    }
    fn apply(self, ws: &mut Workspace, source: DatasetSource) {
        ws.layers = self.layers;
        ws.sources.push(source);
        ws.set_config(self.config);
        ws.prepared = self.prepared;
        ws.edit_version += 1;
        ws.invalidate();
    }
}
/// Read an uploaded dataset and preserve original geometry and attributes.
///
/// # Errors
/// Returns an error for unsupported content, invalid archives or failed vector reads.
pub fn import(source: &DatasetSource, root: &Path) -> Result<Vec<Arc<SourceLayer>>> {
    let mut remaining = archive::EXPANSION_LIMIT;
    import_with_budget(source, root, &mut remaining)
}
fn import_with_budget(
    source: &DatasetSource,
    root: &Path,
    remaining: &mut u64,
) -> Result<Vec<Arc<SourceLayer>>> {
    let path = root.join(&source.path);
    let extension = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let members = if extension == "zip" {
        let directory = root.join(format!("extracted-{}", source.id));
        fs::create_dir_all(&directory)?;
        let entries = archive::extract_with_budget(&path, &directory, false, remaining)?;
        let files = entries
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("shp")))
            .map(|p| directory.join(p))
            .collect::<Vec<_>>();
        if files.is_empty() {
            bail!("ZIP contains no Shapefiles");
        }
        files
    } else if topology_checker_io::registry::for_path(&path)
        .is_some_and(|f| f.native && f.read && f.name != "ESRI Shapefile")
    {
        vec![path]
    } else {
        bail!("upload a supported vector file or a zipped Shapefile");
    };
    let mut layers = Vec::new();
    for member in members {
        let alias = member
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or("dataset");
        for data in read::read_with_attributes(&member, None, alias, true)? {
            layers.push(Arc::new(SourceLayer {
                id: Uuid::new_v4().to_string(),
                dataset: source.id.clone(),
                selector: data.selector,
                member: member.strip_prefix(root)?.to_path_buf(),
                name: data.layer.name().to_string(),
                crs: data.srs.map(|s| s.to_wkt()).transpose()?,
                warnings: data.warnings,
                kinds: kinds(&data.layer),
                layer: Arc::new(data.layer),
                attributes: Arc::new(data.attributes),
            }));
        }
    }
    Ok(layers)
}
/// Add an imported dataset to a workspace that is not shared yet.
///
/// # Errors
/// Returns an error if the prospective working geometry cannot be prepared.
pub fn append(
    ws: &mut Workspace,
    source: DatasetSource,
    layers: Vec<Arc<SourceLayer>>,
) -> Result<()> {
    Addition::prepare(ws.config.clone(), ws.layers.clone(), layers)?.apply(ws, source);
    Ok(())
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedLayer {
    id: String,
    dataset: String,
    #[serde(serialize_with = "archive::serialize_path")]
    member: PathBuf,
    selector: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    sources: Vec<DatasetSource>,
    layers: Vec<SavedLayer>,
    config: Config,
}

#[derive(Debug)]
pub struct ProjectSnapshot {
    pub manifest: Vec<u8>,
    pub files: Vec<(PathBuf, PathBuf)>,
}

/// Save original source files and configuration as a portable project.
///
/// # Errors
/// Returns an error for serialization, filesystem or archive write failures.
pub fn save(ws: &Workspace, output: &Path) -> Result<()> {
    let snapshot = project_parts(ws)?;
    check_project_size(&snapshot)?;
    archive::pack(output, &snapshot.manifest, &snapshot.files)
}
/// Capture project metadata and original file paths without copying geometries.
///
/// # Errors
/// Returns an error when the manifest cannot be serialized.
pub fn project_parts(ws: &Workspace) -> Result<ProjectSnapshot> {
    let manifest = Manifest {
        version: 1,
        sources: ws.sources.clone(),
        layers: ws
            .layers
            .iter()
            .map(|l| SavedLayer {
                id: l.id.clone(),
                dataset: l.dataset.clone(),
                member: l.member.clone(),
                selector: l.selector.clone(),
            })
            .collect(),
        config: ws.config.clone(),
    };
    let files = ws
        .sources
        .iter()
        .map(|s| (s.path.clone(), ws.storage.path().join(&s.path)))
        .collect::<Vec<_>>();
    Ok(ProjectSnapshot {
        manifest: serde_json::to_vec_pretty(&manifest)?,
        files,
    })
}

/// Check that a saved project can be reopened within archive expansion limits.
///
/// # Errors
/// Returns an error for oversized projects or source filesystem failures.
pub fn check_project_size(snapshot: &ProjectSnapshot) -> Result<()> {
    let mut total = snapshot.manifest.len() as u64;
    if total > 8 * 1024 * 1024 {
        bail!("project manifest exceeds 8 MiB");
    }
    for (_, path) in &snapshot.files {
        total = total
            .checked_add(fs::metadata(path)?.len())
            .context("project size overflow")?;
        // Original zipped Shapefiles are embedded, then expanded again on reopen.
        let dataset_directory = path.parent().context("source directory is missing")?;
        let id = dataset_directory
            .file_name()
            .context("source identity is missing")?;
        let root = dataset_directory
            .parent()
            .and_then(Path::parent)
            .context("source root is missing")?;
        let extracted = root.join(format!("extracted-{}", id.to_string_lossy()));
        let mut pending = if extracted.exists() {
            vec![extracted]
        } else {
            Vec::new()
        };
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    pending.push(entry.path());
                } else {
                    total = total
                        .checked_add(entry.metadata()?.len())
                        .context("project size overflow")?;
                }
            }
        }
    }
    if total > archive::EXPANSION_LIMIT {
        bail!("project would expand beyond 2 GiB; save fewer datasets together");
    }
    Ok(())
}
/// Restore a versioned project without retaining previous validation results.
///
/// # Errors
/// Returns an error for unsafe archives, incompatible versions or invalid configuration.
pub fn reopen(path: &Path) -> Result<Workspace> {
    let mut ws = Workspace::new()?;
    let mut remaining = archive::EXPANSION_LIMIT;
    archive::extract_with_budget(path, ws.storage.path(), true, &mut remaining)?;
    let manifest_path = ws.storage.path().join("manifest.json");
    if fs::metadata(&manifest_path)?.len() > 8 * 1024 * 1024 {
        bail!("project manifest is too large");
    }
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.version != 1 {
        bail!("unsupported project version {}", manifest.version);
    }
    let mut datasets = std::collections::HashSet::new();
    let mut paths = std::collections::HashSet::new();
    for source in manifest.sources {
        if uuid::Uuid::parse_str(&source.id).is_err() && source.id != "example" {
            bail!("invalid project dataset identity");
        }
        let filename = archive::safe_path(&source.filename)?;
        if filename.components().count() != 1
            || source.path != PathBuf::from("sources").join(&source.id).join(&filename)
        {
            bail!("invalid project source location");
        }
        archive::safe_path(
            source
                .path
                .to_str()
                .context("invalid project source path")?,
        )?;
        if source.path == Path::new("manifest.json")
            || !datasets.insert(source.id.clone())
            || !paths.insert(source.path.clone())
        {
            bail!("invalid or duplicate project source");
        }
        let layers = import_with_budget(&source, ws.storage.path(), &mut remaining)?;
        for mut layer in layers {
            let saved = manifest
                .layers
                .iter()
                .find(|s| {
                    s.dataset == layer.dataset
                        && s.member == layer.member
                        && s.selector == layer.selector
                })
                .context("project layer selection no longer exists")?;
            Arc::get_mut(&mut layer).unwrap().id = saved.id.clone();
            ws.layers.push(layer);
        }
        ws.sources.push(source);
    }
    if ws.layers.len() != manifest.layers.len() {
        bail!("project layer manifest mismatch");
    }
    let mut config = manifest.config;
    let old = config.clone(); // Preserve a saved explicit tolerance on reopen.
    normalize_config(&mut config, &old, &ws.layers)?;
    ws.initialize(config)?;
    Ok(ws)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{ZipWriter, write::SimpleFileOptions};

    #[test]
    fn project_paths_use_portable_separators_and_reopen() {
        let mut ws = Workspace::new().unwrap();
        let incoming = tempfile::tempdir().unwrap();
        let path = incoming.path().join("streets.geojson");
        fs::write(&path, include_str!("../example.geojson")).unwrap();
        let pending = PendingImport::copy(ws.root(), "streets.geojson".into(), &path).unwrap();
        let source = pending.commit();
        let layers = import(&source, ws.root()).unwrap();
        append(&mut ws, source.clone(), layers).unwrap();
        let snapshot = project_parts(&ws).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&snapshot.manifest).unwrap();
        let expected = format!("sources/{}/streets.geojson", source.id);
        assert_eq!(manifest["sources"][0]["path"], expected);
        assert_eq!(manifest["layers"][0]["member"], expected);
        let output = incoming.path().join("project.zip");
        save(&ws, &output).unwrap();
        let restored = reopen(&output).unwrap();
        assert_eq!(restored.sources()[0].id, source.id);
        assert_eq!(restored.config(), ws.config());
        assert_eq!(restored.layers()[0].id, ws.layers()[0].id);
    }

    #[test]
    fn failed_import_removes_original_and_partially_extracted_files() {
        let ws = Workspace::new().unwrap();
        let incoming = tempfile::tempdir().unwrap();
        let path = incoming.path().join("bad.zip");
        let mut zip = ZipWriter::new(fs::File::create(&path).unwrap());
        zip.start_file("data.dbf", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"partial extraction").unwrap();
        zip.start_file("unsupported.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"reject this entry").unwrap();
        zip.finish().unwrap();
        let pending = PendingImport::copy(ws.storage.path(), "bad.zip".into(), &path).unwrap();
        let original = ws.storage.path().join(&pending.source.path);
        let extracted = ws
            .storage
            .path()
            .join(format!("extracted-{}", pending.source.id));
        assert!(import(&pending.source, ws.storage.path()).is_err());
        assert!(original.exists());
        assert!(extracted.join("data.dbf").exists());
        drop(pending);
        assert!(!original.parent().unwrap().exists());
        assert!(!extracted.exists());
        assert!(ws.layers.is_empty());
        assert!(ws.sources.is_empty());
        assert_eq!(ws.revision, 0);
    }

    #[test]
    fn abandoned_prepared_import_rolls_back_but_committed_import_keeps_original() {
        let mut ws = Workspace::new().unwrap();
        let incoming = tempfile::tempdir().unwrap();
        let path = incoming.path().join("data.geojson");
        fs::write(&path, include_str!("../example.geojson")).unwrap();
        let prepare_import = |ws: &Workspace| {
            let snapshot = ws.snapshot();
            let pending =
                PendingImport::copy(&snapshot.root, "data.geojson".into(), &path).unwrap();
            let layers = import(&pending.source, &snapshot.root).unwrap();
            let addition =
                Addition::prepare(snapshot.config.clone(), snapshot.layers.clone(), layers)
                    .unwrap();
            let original = snapshot.root.join(&pending.source.path);
            (snapshot, pending, addition, original)
        };
        let (snapshot, pending, addition, original) = prepare_import(&ws);
        // A concurrent configuration change rejects an otherwise valid import.
        let mut changed = ws.config().clone();
        changed.tolerance = 1.;
        ws.commit_config(&ws.snapshot(), changed, Arc::clone(ws.prepared()))
            .unwrap();
        assert!(ws.commit_import(&snapshot, addition, pending).is_err());
        assert!(!original.exists());
        assert!(ws.sources.is_empty());
        assert_eq!(ws.revision, 1);

        let (snapshot, pending, addition, original) = prepare_import(&ws);
        ws.commit_import(&snapshot, addition, pending).unwrap();
        assert!(original.exists());
        assert_eq!(ws.sources.len(), 1);
        assert_eq!(ws.layers.len(), 1);
        assert_eq!(ws.revision, 2);
    }

    #[test]
    fn appearance_edits_keep_results_and_late_results_are_stale() {
        let mut ws = Workspace::new().unwrap();
        let incoming = tempfile::tempdir().unwrap();
        let path = incoming.path().join("streets.geojson");
        fs::write(&path, include_str!("../example.geojson")).unwrap();
        let snapshot = ws.snapshot();
        let pending = PendingImport::copy(&snapshot.root, "streets.geojson".into(), &path).unwrap();
        let layers = import(&pending.source, &snapshot.root).unwrap();
        let addition =
            Addition::prepare(snapshot.config.clone(), snapshot.layers.clone(), layers).unwrap();
        ws.commit_import(&snapshot, addition, pending).unwrap();
        let mut config = ws.config().clone();
        config.checks.push(Check {
            rule: topology_checker_core::Rule::LineMustNotHaveDangles,
            layer: ws.layers[0].id.clone(),
            other: None,
            enabled: true,
        });
        ws.commit_config(&ws.snapshot(), config, Arc::clone(ws.prepared()))
            .unwrap();
        let revision = ws.revision;

        let ticket = ws.start_job();
        assert!(ws.job_running());
        let mut appearance = ws.config().clone();
        appearance.view.basemap = true;
        appearance.layers[0].color = "#000000".into();
        ws.commit_config(&ws.snapshot(), appearance, Arc::clone(ws.prepared()))
            .unwrap();
        assert_eq!(ws.revision, revision, "appearance does not change results");
        let result =
            ValidationResult::run(ticket.revision, ticket.config.clone(), &ticket.prepared);
        ws.finish_job(&ticket, result);
        assert_eq!(ws.poll().job.unwrap().status, JobStatus::Completed);
        assert!(ws.result().is_some());

        let previous = ws.start_job();
        let current = ws.start_job();
        let result = ValidationResult::run(
            previous.revision,
            previous.config.clone(),
            &previous.prepared,
        );
        ws.finish_job(&previous, result);
        assert_eq!(ws.poll().job.unwrap().id, current.id);
        assert!(ws.job_running());
        let result =
            ValidationResult::run(current.revision, current.config.clone(), &current.prepared);
        ws.finish_job(&current, result);
        assert_eq!(ws.poll().job.unwrap().status, JobStatus::Completed);

        let ticket = ws.start_job();
        let result =
            ValidationResult::run(ticket.revision, ticket.config.clone(), &ticket.prepared);
        let mut tolerance = ws.config().clone();
        tolerance.tolerance = 0.5;
        ws.commit_config(&ws.snapshot(), tolerance, Arc::clone(ws.prepared()))
            .unwrap();
        ws.finish_job(&ticket, result);
        let poll = ws.poll();
        assert_eq!(poll.job.unwrap().status, JobStatus::Stale);
        assert!(poll.result.is_none());
        assert!(!ws.job_running());
    }
}
