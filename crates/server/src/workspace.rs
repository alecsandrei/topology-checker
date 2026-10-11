use crate::{archive, model::*};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use topology_checker_io::read;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub revision: u64,
    pub status: String,
    pub message: Option<String>,
    pub config: Config,
}
#[derive(Debug)]
pub struct Workspace {
    pub id: String,
    pub storage: TempDir,
    pub sources: Vec<DatasetSource>,
    pub layers: Vec<Arc<SourceLayer>>,
    pub prepared: Vec<Arc<IndexedLayer>>,
    pub config: Config,
    pub revision: u64,
    pub version: u64,
    pub job: Option<Job>,
    pub result: Option<Arc<ValidationResult>>,
}
impl Workspace {
    /// Create a temporary local workspace.
    ///
    /// # Errors
    /// Returns an error when temporary storage cannot be created.
    pub fn new() -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            storage: tempfile::tempdir()?,
            sources: vec![],
            layers: vec![],
            prepared: vec![],
            config: Config::default(),
            revision: 0,
            version: 0,
            job: None,
            result: None,
        })
    }
    pub fn json(&self) -> Value {
        json!({"id":self.id,"revision":self.revision,"config":self.config,"working_crs_info":self.config.working_crs.as_deref().and_then(|c|crs_info(c).ok()),"datasets":self.sources.iter().map(|s|json!({"id":s.id,"filename":s.filename,"provenance":s.provenance})).collect::<Vec<_>>(),"layers":layer_metadata(&self.layers,&self.prepared,&self.config),"job":self.job,"result":self.result.as_ref().map(|r|r.summary())})
    }
    pub fn invalidate(&mut self) {
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

#[derive(Debug)]
pub(crate) struct Addition {
    config: Config,
    layers: Vec<Arc<SourceLayer>>,
    prepared: Vec<Arc<IndexedLayer>>,
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
    pub fn commit(self, ws: &mut Workspace, source: DatasetSource) {
        ws.layers = self.layers;
        ws.sources.push(source);
        ws.config = self.config;
        ws.prepared = self.prepared;
        ws.version += 1;
        ws.invalidate();
    }
}
/// Read an uploaded dataset and preserve original geometry and attributes.
///
/// # Errors
/// Returns an error for unsupported content, invalid archives or failed GDAL reads.
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
    } else if matches!(extension.as_str(), "geojson" | "json" | "gpkg" | "fgb") {
        vec![path]
    } else {
        bail!("upload GeoJSON, GeoPackage, FlatGeobuf or a zipped Shapefile");
    };
    let mut layers = Vec::new();
    for member in members {
        let alias = member
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or("dataset");
        for data in read::read_with_attributes(&member, None, alias, true)? {
            let features = data
                .layer
                .features()
                .map(|(fid, g)| (fid, g.clone()))
                .collect::<Vec<_>>();
            let kinds = kinds(&features);
            layers.push(Arc::new(SourceLayer {
                id: Uuid::new_v4().to_string(),
                dataset: source.id.clone(),
                selector: data.selector,
                member: member.strip_prefix(root)?.to_path_buf(),
                name: data.layer.name().to_string(),
                crs: data.srs.map(|s| s.to_wkt()).transpose()?,
                warnings: data.warnings,
                features,
                attributes: Arc::new(data.attributes),
                kinds,
            }));
        }
    }
    Ok(layers)
}
/// Add an imported dataset to a workspace.
///
/// # Errors
/// Returns an error if the prospective working geometry cannot be prepared.
pub fn append(
    ws: &mut Workspace,
    source: DatasetSource,
    layers: Vec<Arc<SourceLayer>>,
) -> Result<()> {
    Addition::prepare(ws.config.clone(), ws.layers.clone(), layers)?.commit(ws, source);
    Ok(())
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedLayer {
    id: String,
    dataset: String,
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
    ws.prepared = prepare(&config, &ws.layers)?;
    ws.config = config;
    Ok(ws)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{ZipWriter, write::SimpleFileOptions};

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
        let pending = PendingImport::copy(ws.storage.path(), "data.geojson".into(), &path).unwrap();
        let layers = import(&pending.source, ws.storage.path()).unwrap();
        let _addition = Addition::prepare(ws.config.clone(), ws.layers.clone(), layers).unwrap();
        let original = ws.storage.path().join(&pending.source.path);
        // A concurrent configuration change can reject an otherwise valid import.
        ws.version += 1;
        drop(pending);
        assert!(!original.exists());
        assert!(ws.sources.is_empty());

        let pending = PendingImport::copy(ws.storage.path(), "data.geojson".into(), &path).unwrap();
        let layers = import(&pending.source, ws.storage.path()).unwrap();
        let addition = Addition::prepare(ws.config.clone(), ws.layers.clone(), layers).unwrap();
        let original = ws.storage.path().join(&pending.source.path);
        addition.commit(&mut ws, pending.commit());
        assert!(original.exists());
        assert_eq!(ws.sources.len(), 1);
        assert_eq!(ws.layers.len(), 1);
        assert_eq!(ws.revision, 1);
    }
}
