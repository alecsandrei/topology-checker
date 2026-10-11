//! Typed HTTP responses. `cargo test` exports their TypeScript bindings to `web/src/generated`.
use crate::model::Config;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use topology_checker_core::{GeometryKind, Rule, ViolationKind};
use ts_rs::TS;

/// A GeoJSON geometry in working or display coordinates.
pub type GeoJson = geojson::Geometry;
/// Feature attributes, with integers beyond JavaScript's safe range as strings.
pub type Attributes = Map<String, Value>;

/// A failed request.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ErrorBody {
    pub error: String,
}

/// A rule catalog entry.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct RuleInfo {
    pub name: Rule,
    pub description: String,
    pub primary: GeometryKind,
    pub secondary: Option<GeometryKind>,
}

/// A parsed working CRS with its default tolerance.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CrsInfo {
    pub definition: String,
    pub label: String,
    pub units: Option<String>,
    pub default_tolerance: f64,
}

/// Source and licensing details of a bundled dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Provenance {
    pub provider: String,
    pub source_urls: Vec<String>,
    pub revision: String,
    pub sha256: String,
    pub download_date: String,
    pub license: String,
    pub license_url: String,
    pub grouping: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<ProvenanceInput>>,
}
/// A source file that contributed to a generated dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProvenanceInput {
    pub filename: String,
    pub sha256: String,
}

/// A bundled example workspace.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct Example {
    pub id: String,
    pub title: String,
    pub description: String,
    pub note: String,
    pub datasets: Vec<ExampleDataset>,
}
/// A dataset and its default checks within a bundled example.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ExampleDataset {
    pub filename: String,
    pub asset: String,
    pub feature_count: usize,
    pub imported_feature_count: usize,
    pub geometry_types: Vec<String>,
    pub provenance: Provenance,
    pub visible: bool,
    pub checks: Vec<Rule>,
}

/// An imported dataset.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, rename = "Dataset")]
pub struct DatasetView {
    pub id: String,
    pub filename: String,
    pub provenance: Option<Provenance>,
}

/// An imported layer with its working-CRS and display extents.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, rename = "Layer")]
pub struct LayerView {
    pub id: String,
    pub dataset: String,
    pub selector: String,
    pub name: String,
    pub feature_count: usize,
    pub kinds: Vec<GeometryKind>,
    pub source_crs: Option<String>,
    pub warnings: Vec<String>,
    pub extent: Option<[f64; 4]>,
    pub display_extent: Option<[f64; 4]>,
}

/// Lifecycle of a validation job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum JobStatus {
    Running,
    Completed,
    Stale,
    Failed,
}
/// The latest validation job and the configuration it checks.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Job {
    pub id: String,
    pub revision: u64,
    pub status: JobStatus,
    pub message: Option<String>,
    pub config: Config,
}
/// An accepted validation request.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct RunAccepted {
    pub id: String,
    pub revision: u64,
    pub status: JobStatus,
}
/// The job status and any current result summary.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct JobPoll {
    pub job: Option<Job>,
    pub result: Option<Summary>,
}

/// Violation counts for one registered rule.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct RuleSummary {
    pub rule: Rule,
    pub layer: String,
    pub other: Option<String>,
    pub counts: BTreeMap<ViolationKind, usize>,
    pub total: usize,
}
/// A completed validation without its violation geometries.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Summary {
    pub revision: u64,
    pub config: Config,
    pub valid: bool,
    pub total: usize,
    pub warnings: Vec<String>,
    pub rules: Vec<RuleSummary>,
}

/// A feature contributing to a violation. FIDs are strings to keep 64-bit precision.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, rename = "Ref")]
pub struct SourceRef {
    pub layer: String,
    pub name: String,
    pub dataset: String,
    pub selector: String,
    pub fid: String,
    pub part: Option<u32>,
    pub attributes: Option<Attributes>,
}
/// One aggregated violation in deterministic report order.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Violation {
    pub id: usize,
    pub rule: Rule,
    pub category: ViolationKind,
    #[ts(type = "import(\"geojson\").Geometry")]
    pub geometry: GeoJson,
    pub origin: Option<SourceRef>,
    pub destination: Option<SourceRef>,
}
/// A page of violations.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Page {
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub violations: Vec<Violation>,
}

/// The GeoJSON feature type tag.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(export)]
pub enum FeatureTag {
    Feature,
}
/// A source feature for the map or inspector.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MapFeature {
    #[serde(rename = "type")]
    pub kind: FeatureTag,
    pub id: String,
    #[ts(type = "import(\"geojson\").Geometry")]
    pub geometry: GeoJson,
    pub properties: Option<Attributes>,
    pub layer: String,
}
/// The GeoJSON feature collection type tag.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(export)]
pub enum CollectionTag {
    FeatureCollection,
}
/// A bounded preview of the features intersecting a viewport.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Viewport {
    #[serde(rename = "type")]
    pub kind: CollectionTag,
    pub features: Vec<MapFeature>,
    pub incomplete: bool,
    pub coordinates: usize,
    pub feature_limit: usize,
    pub coordinate_limit: usize,
}

/// A workspace with its configuration, layers and latest job.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, rename = "Workspace")]
pub struct WorkspaceView {
    pub id: String,
    pub revision: u64,
    pub config: Config,
    pub working_crs_info: Option<CrsInfo>,
    pub datasets: Vec<DatasetView>,
    pub layers: Vec<LayerView>,
    pub job: Option<Job>,
    pub result: Option<Summary>,
}

/// Export bindings for types that no exported response references.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_registry_bindings() {
        topology_checker_io::registry::Format::export_all(&ts_rs::Config::from_env()).unwrap();
    }
}
