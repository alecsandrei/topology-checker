use crate::dto::{
    Attributes, CollectionTag, CrsInfo, FeatureTag, GeoJson, LayerView, MapFeature, Page,
    Provenance, RuleInfo, RuleSummary, SourceRef, Summary, Viewport, Violation,
};
use anyhow::{Context, Result, bail};
use geo::{BoundingRect, CoordsIter, Geometry};
use rstar::{AABB, RTree, RTreeObject};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};
use topology_checker_core::{GeometryKind, Layer, Report, Rule, Topology};
use topology_checker_io::Transform as CoordTransform;
use topology_checker_io::{crs, read};
use ts_rs::TS;

pub const FEATURE_LIMIT: usize = 10_000;
pub const COORDINATE_LIMIT: usize = 200_000;
pub const PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct Check {
    pub rule: Rule,
    pub layer: String,
    pub other: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}
fn yes() -> bool {
    true
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct LayerSettings {
    pub id: String,
    pub selected: bool,
    pub visible: bool,
    pub color: String,
    pub source_crs: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ViewSettings {
    pub basemap: bool,
    pub center: Option<[f64; 2]>,
    pub resolution: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct Config {
    pub working_crs: Option<String>,
    pub tolerance: f64,
    pub layers: Vec<LayerSettings>,
    pub checks: Vec<Check>,
    #[serde(default)]
    pub view: ViewSettings,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            working_crs: None,
            tolerance: 0.,
            layers: vec![],
            checks: vec![],
            view: ViewSettings::default(),
        }
    }
}
/// Layer settings that select and georeference working geometry: (id, selected, source CRS).
type LayerKey<'a> = Vec<(&'a str, bool, Option<&'a str>)>;
impl Config {
    fn layer_key(&self) -> LayerKey<'_> {
        let mut key = self
            .layers
            .iter()
            .map(|l| (l.id.as_str(), l.selected, l.source_crs.as_deref()))
            .collect::<Vec<_>>();
        key.sort_unstable();
        key
    }
    /// Whether `other` produces different working geometry, so layers must be prepared again.
    #[must_use]
    pub fn geometry_differs(&self, other: &Self) -> bool {
        self.working_crs != other.working_crs || self.layer_key() != other.layer_key()
    }
    /// Whether `other` can produce different validation results. Colours, visibility
    /// and the map view do not.
    #[must_use]
    pub fn validation_differs(&self, other: &Self) -> bool {
        self.geometry_differs(other)
            || self.tolerance != other.tolerance
            || self.checks != other.checks
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetSource {
    pub id: String,
    pub filename: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}
#[derive(Debug)]
pub struct SourceLayer {
    pub id: String,
    pub dataset: String,
    pub selector: String,
    pub member: PathBuf,
    pub name: String,
    pub crs: Option<String>,
    pub warnings: Vec<String>,
    /// Original geometry, shared with prepared layers that need no reprojection.
    pub layer: Arc<Layer>,
    pub attributes: Arc<Vec<Attributes>>,
    pub kinds: Vec<GeometryKind>,
}
#[derive(Debug, Clone)]
struct Entry {
    ordinal: usize,
    bounds: AABB<[f64; 2]>,
    coordinates: usize,
}
impl RTreeObject for Entry {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.bounds
    }
}
/// A layer in the working CRS, indexed for previews. Its `Arc<Layer>` is shared with
/// every validation topology, so cleaned parts, indexes and unions are built once.
#[derive(Debug)]
pub struct IndexedLayer {
    pub source: Arc<SourceLayer>,
    pub layer: Arc<Layer>,
    index: RTree<Entry>,
    ids: HashMap<i64, usize>,
    pub extent: Option<[f64; 4]>,
    /// The extent in Web Mercator, when a working CRS is assigned.
    pub display_extent: Option<[f64; 4]>,
}
impl IndexedLayer {
    /// Index complete geometries by extent and feature identity.
    ///
    /// # Errors
    /// Returns an error for duplicate FIDs or non-finite coordinates.
    pub fn new(
        source: Arc<SourceLayer>,
        layer: Arc<Layer>,
        working_crs: Option<&str>,
    ) -> Result<Self> {
        let mut entries = Vec::new();
        let mut extent: Option<[f64; 4]> = None;
        let mut ids = HashMap::new();
        for (i, (fid, g)) in layer.features().enumerate() {
            if ids.insert(fid, i).is_some() {
                bail!("duplicate FID {fid} in {}", source.name);
            }
            if g.coords_iter()
                .any(|c| !c.x.is_finite() || !c.y.is_finite())
            {
                bail!("non-finite coordinates in {}, FID {fid}", source.name);
            }
            if let Some(r) = g.bounding_rect() {
                let bounds = [r.min().x, r.min().y, r.max().x, r.max().y];
                extent = Some(extent.map_or(bounds, |e| {
                    [
                        e[0].min(bounds[0]),
                        e[1].min(bounds[1]),
                        e[2].max(bounds[2]),
                        e[3].max(bounds[3]),
                    ]
                }));
                entries.push(Entry {
                    ordinal: i,
                    bounds: AABB::from_corners([bounds[0], bounds[1]], [bounds[2], bounds[3]]),
                    coordinates: g.coords_count(),
                });
            }
        }
        let display_extent = extent.and_then(|e| {
            let to = crs::transform_between(working_crs?, "EPSG:3857").ok()?;
            to.transform_bounds(&e, 21).ok()
        });
        Ok(Self {
            source,
            layer,
            index: RTree::bulk_load(entries),
            ids,
            extent,
            display_extent,
        })
    }
    fn geometry(&self, ordinal: usize) -> (i64, &Geometry) {
        self.layer
            .feature(ordinal)
            .expect("indexed ordinals belong to the layer")
    }
    /// Look up one complete feature in working or display coordinates.
    ///
    /// # Errors
    /// Returns an error for an unknown working CRS or a failed display transformation.
    pub fn feature(
        &self,
        fid: i64,
        working_crs: Option<&str>,
        display: bool,
    ) -> Result<Option<MapFeature>> {
        let Some(&ordinal) = self.ids.get(&fid) else {
            return Ok(None);
        };
        let g = self.geometry(ordinal).1;
        let transformed;
        let g = if display {
            let working = working_crs.context("working CRS is unknown")?;
            let to_display = crs::transform_between(working, "EPSG:3857")?;
            transformed = crs::transform(g, &to_display)?;
            &transformed
        } else {
            g
        };
        Ok(Some(self.map_feature(ordinal, g)))
    }
    fn map_feature(&self, ordinal: usize, g: &Geometry) -> MapFeature {
        MapFeature {
            kind: FeatureTag::Feature,
            id: self.geometry(ordinal).0.to_string(),
            geometry: geometry(g),
            properties: self.source.attributes.get(ordinal).cloned(),
            layer: self.source.id.clone(),
        }
    }
    /// Query a bounded spatial preview in working or display coordinates.
    ///
    /// # Errors
    /// Returns an error for unknown CRS or failed display transformations.
    pub fn viewport(
        &self,
        bounds: [f64; 4],
        working_crs: Option<&str>,
        display: bool,
    ) -> Result<Viewport> {
        let to_display = if display {
            let working = working_crs.context("assign a working CRS for map display")?;
            Some((
                crs::transform_between("EPSG:3857", working)?,
                crs::transform_between(working, "EPSG:3857")?,
            ))
        } else {
            None
        };
        let bounds = if let Some((to_working, _)) = &to_display {
            to_working.transform_bounds(&bounds, 21)?
        } else {
            bounds
        };
        let to_display = to_display.map(|(_, to_display)| to_display);
        let query = AABB::from_corners([bounds[0], bounds[1]], [bounds[2], bounds[3]]);
        let mut output = Vec::new();
        let mut coordinates = 0;
        let mut incomplete = false;
        for entry in self.index.locate_in_envelope_intersecting(&query) {
            if output.len() >= FEATURE_LIMIT {
                incomplete = true;
                break;
            }
            if coordinates + entry.coordinates > COORDINATE_LIMIT {
                incomplete = true;
                continue;
            }
            let g = self.geometry(entry.ordinal).1;
            let transformed;
            let g = if let Some(t) = &to_display {
                transformed = crs::transform(g, t)?;
                &transformed
            } else {
                g
            };
            output.push(self.map_feature(entry.ordinal, g));
            coordinates += entry.coordinates;
        }
        Ok(Viewport {
            kind: CollectionTag::FeatureCollection,
            features: output,
            incomplete,
            coordinates,
            feature_limit: FEATURE_LIMIT,
            coordinate_limit: COORDINATE_LIMIT,
        })
    }
}
/// Convert a geometry to GeoJSON without changing its coordinates.
pub fn geometry(g: &Geometry) -> GeoJson {
    geojson::Geometry::new(geojson::Value::from(g))
}
/// Return the distinct geometry families in a layer, looking inside collections.
pub fn kinds(layer: &Layer) -> Vec<GeometryKind> {
    fn add(g: &Geometry, out: &mut Vec<GeometryKind>) {
        if let Geometry::GeometryCollection(c) = g {
            for child in c {
                add(child, out);
            }
        } else {
            let kind = GeometryKind::of(g);
            if !out.contains(&kind) {
                out.push(kind);
            }
        }
    }
    let mut out = Vec::new();
    for (_, g) in layer.features() {
        add(g, &mut out);
    }
    out
}
/// Validate configuration and apply CRS-dependent defaults.
///
/// # Errors
/// Returns an error for invalid tolerances, layers, geometry families or CRS definitions.
pub fn normalize_config(
    config: &mut Config,
    old: &Config,
    sources: &[Arc<SourceLayer>],
) -> Result<()> {
    if !config.tolerance.is_finite() || config.tolerance < 0. {
        bail!("tolerance must be finite and nonnegative");
    }
    if config
        .view
        .center
        .is_some_and(|v| v.iter().any(|x| !x.is_finite()))
        || config
            .view
            .resolution
            .is_some_and(|v| !v.is_finite() || v <= 0.)
    {
        bail!("invalid view coordinates");
    }
    let srs = config.working_crs.as_deref().map(crs::parse).transpose()?;
    if srs.is_none() {
        config.view.basemap = false;
    }
    if config.working_crs != old.working_crs {
        config.tolerance = read::default_tolerance(srs.as_ref());
        config.view.center = None;
        config.view.resolution = None;
    }
    if config.layers.len() != sources.len() {
        bail!("configuration must contain all imported layers");
    }
    let mut seen = HashSet::new();
    for l in &config.layers {
        if !seen.insert(&l.id) || !sources.iter().any(|s| s.id == l.id) {
            bail!("unknown or duplicate layer {}", l.id);
        }
        if l.color.len() != 7
            || !l.color.starts_with('#')
            || !l.color[1..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            bail!("invalid layer color");
        }
        if let Some(def) = &l.source_crs {
            crs::parse(def)?;
        }
    }
    for c in config.checks.iter().filter(|c| c.enabled) {
        c.rule.check_input_count(c.other.is_some())?;
        if c.other.as_ref() == Some(&c.layer) {
            bail!("{} requires two distinct input layers", c.rule);
        }
        let (a, b) = c.rule.inputs();
        for (id, kind) in [(Some(&c.layer), Some(a)), (c.other.as_ref(), b)] {
            if let (Some(id), Some(kind)) = (id, kind) {
                let s = sources
                    .iter()
                    .find(|s| &s.id == id)
                    .context("unknown rule layer")?;
                if !config.layers.iter().any(|l| &l.id == id && l.selected) {
                    bail!("rule input is not selected");
                }
                if !s.kinds.iter().all(|k| k.satisfies(kind)) {
                    bail!("{} requires {kind:?} input", c.rule);
                }
            }
        }
    }
    Ok(())
}
/// Selected layers in the working CRS, with a validation topology reused by every run
/// at the same tolerance. A configuration that changes working geometry prepares a new
/// instance, so the cached topology can never see stale layers.
#[derive(Debug, Default)]
pub struct Prepared {
    pub layers: Vec<Arc<IndexedLayer>>,
    topology: Mutex<Option<Topology>>,
}
impl Prepared {
    /// Find a selected layer by its workspace ID.
    #[must_use]
    pub fn layer(&self, id: &str) -> Option<&Arc<IndexedLayer>> {
        self.layers.iter().find(|l| l.source.id == id)
    }
}

/// Transform original layers into the working CRS and index their complete geometries.
/// Layers already in the working CRS share their original geometry.
///
/// # Errors
/// Returns an error for missing source CRSs, failed transforms or invalid geometries.
pub fn prepare(config: &Config, sources: &[Arc<SourceLayer>]) -> Result<Arc<Prepared>> {
    let target = config.working_crs.as_deref().map(crs::parse).transpose()?;
    let layers = sources
        .iter()
        .filter(|s| config.layers.iter().any(|l| l.id == s.id && l.selected))
        .map(|s| {
            let settings = config.layers.iter().find(|l| l.id == s.id).unwrap();
            let source = settings.source_crs.as_deref().or(s.crs.as_deref());
            let transform = if let Some(target) = &target {
                let source = crs::parse(source.with_context(|| {
                    format!("assign a source CRS to {} before reprojection", s.name)
                })?)?;
                if &source == target {
                    None
                } else {
                    Some(CoordTransform::new(&source, target)?)
                }
            } else {
                None
            };
            let layer = if let Some(t) = &transform {
                let features = s
                    .layer
                    .features()
                    .map(|(fid, g)| {
                        Ok((
                            fid,
                            crs::transform(g, t)
                                .with_context(|| format!("transforming {}, FID {fid}", s.name))?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Arc::new(Layer::from_features(&s.name, features))
            } else {
                Arc::clone(&s.layer)
            };
            Ok(Arc::new(IndexedLayer::new(
                Arc::clone(s),
                layer,
                config.working_crs.as_deref(),
            )?))
        })
        .collect::<Result<_>>()?;
    Ok(Arc::new(Prepared {
        layers,
        topology: Mutex::default(),
    }))
}
#[derive(Debug)]
pub struct ValidationResult {
    pub revision: u64,
    pub config: Config,
    pub layers: Vec<Arc<IndexedLayer>>,
    pub report: Report,
    /// Core, import and CRS warnings as display text.
    pub warnings: Vec<String>,
    rows: Vec<(usize, usize)>,
}
impl ValidationResult {
    /// Validate an immutable workspace configuration with the existing core, reusing
    /// the prepared topology and its line graphs while the tolerance is unchanged.
    ///
    /// # Errors
    /// Returns an error for invalid rule inputs or core validation failures.
    pub fn run(revision: u64, config: Config, prepared: &Prepared) -> Result<Self> {
        if !config.checks.iter().any(|c| c.enabled) {
            bail!("enable at least one rule");
        }
        // Registration is transactional and validation only reads the topology, so a
        // panic elsewhere cannot leave it inconsistent.
        let mut cached = prepared
            .topology
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if cached
            .as_ref()
            .is_none_or(|t| t.tolerance() != config.tolerance)
        {
            let mut topology = Topology::new(config.tolerance)?;
            for l in &prepared.layers {
                topology.add_layer(Arc::clone(&l.layer));
            }
            *cached = Some(topology);
        }
        let topology = cached.as_mut().expect("topology prepared above");
        topology.clear_rules();
        let ids = prepared
            .layers
            .iter()
            .zip(topology.layers())
            .map(|(l, (id, _))| (&l.source.id, id))
            .collect::<HashMap<_, _>>();
        let layers = prepared.layers.clone();
        for c in config.checks.iter().filter(|c| c.enabled) {
            topology.add_rule(
                c.rule,
                *ids.get(&c.layer).context("missing primary layer")?,
                c.other
                    .as_ref()
                    .map(|id| ids.get(id).copied().context("missing secondary layer"))
                    .transpose()?,
            )?;
        }
        let report = topology.validate()?;
        drop(cached);
        let mut warnings = report
            .warnings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        for l in &layers {
            warnings.extend(l.source.warnings.iter().cloned());
        }
        if config.working_crs.is_none() {
            warnings.push(
                "Raw XY mode: input coordinates are used directly without CRS transformation"
                    .into(),
            );
        }
        if config
            .working_crs
            .as_deref()
            .map(crs::parse)
            .transpose()?
            .is_some_and(|s| s.is_geographic())
        {
            warnings.push("Tolerance is in angular units; calculations are planar".into());
        }
        let rows = report
            .rules
            .iter()
            .enumerate()
            .flat_map(|(r, rule)| (0..rule.violations.len()).map(move |e| (r, e)))
            .collect();
        Ok(Self {
            revision,
            config,
            layers,
            report,
            warnings,
            rows,
        })
    }
    fn reference(&self, r: Option<topology_checker_core::FeatureRef>) -> Option<SourceRef> {
        r.map(|r| {
            let l = &self.layers[r.layer.index()];
            SourceRef {
                layer: l.source.id.clone(),
                name: l.source.name.clone(),
                dataset: l.source.dataset.clone(),
                selector: l.source.selector.clone(),
                fid: r.fid.to_string(),
                part: r.part,
                attributes: l
                    .ids
                    .get(&r.fid)
                    .and_then(|&i| l.source.attributes.get(i))
                    .cloned(),
            }
        })
    }
    /// Serialize a single aggregated violation and signed source references.
    ///
    /// # Errors
    /// Returns an error for an unknown violation or failed display transformation.
    pub fn violation(&self, index: usize, display: bool) -> Result<Violation> {
        let &(rule, error) = self.rows.get(index).context("unknown violation")?;
        let e = &self.report.rules[rule].violations[error];
        let transform = if display {
            Some(crs::transform_between(
                self.config
                    .working_crs
                    .as_deref()
                    .context("working CRS is unknown")?,
                "EPSG:3857",
            )?)
        } else {
            None
        };
        let g = if let Some(t) = &transform {
            crs::transform(&e.geometry, t)?
        } else {
            e.geometry.clone()
        };
        Ok(Violation {
            id: index,
            rule: e.rule,
            category: e.kind,
            geometry: geometry(&g),
            origin: self.reference(e.origin),
            destination: self.reference(e.destination),
        })
    }
    pub fn summary(&self) -> Summary {
        Summary {
            revision: self.revision,
            config: self.config.clone(),
            valid: self.report.is_valid(),
            total: self.report.violation_count(),
            warnings: self.warnings.clone(),
            rules: self
                .report
                .rules
                .iter()
                .map(|r| RuleSummary {
                    rule: r.spec.rule,
                    layer: self.layers[r.spec.layer.index()].source.id.clone(),
                    other: r
                        .spec
                        .other
                        .map(|id| self.layers[id.index()].source.id.clone()),
                    counts: r.counts.clone(),
                    total: r.violations.len(),
                })
                .collect(),
        }
    }
    /// Return up to 100 violations in deterministic core report order.
    ///
    /// # Errors
    /// Returns an error for page offset overflow or violation serialization failures.
    pub fn page(&self, page: usize) -> Result<Page> {
        let start = page.checked_mul(PAGE_SIZE).context("page out of range")?;
        let violations = (start
            ..(start.saturating_add(PAGE_SIZE)).min(self.report.violation_count()))
            .map(|i| self.violation(i, false))
            .collect::<Result<Vec<_>>>()?;
        Ok(Page {
            page,
            page_size: PAGE_SIZE,
            total: self.report.violation_count(),
            violations,
        })
    }
    /// Serialize the complete report with its configuration snapshot.
    ///
    /// # Errors
    /// Returns an error if a violation cannot be serialized.
    pub fn complete_json(&self) -> Result<serde_json::Value> {
        #[derive(Serialize)]
        struct Complete {
            #[serde(flatten)]
            summary: Summary,
            violations: Vec<Violation>,
        }
        Ok(serde_json::to_value(Complete {
            summary: self.summary(),
            violations: (0..self.report.violation_count())
                .map(|i| self.violation(i, false))
                .collect::<Result<_>>()?,
        })?)
    }
}
pub fn catalog() -> Vec<RuleInfo> {
    Rule::ALL
        .iter()
        .map(|&rule| {
            let (primary, secondary) = rule.inputs();
            RuleInfo {
                name: rule,
                description: rule.description().into(),
                primary,
                secondary,
            }
        })
        .collect()
}
/// Describe a working CRS, its units and default tolerance.
///
/// # Errors
/// Returns an error for invalid CRS definitions or failed CRS metadata conversion.
pub fn crs_info(def: &str) -> Result<CrsInfo> {
    let s = crs::parse(def)?;
    Ok(CrsInfo {
        definition: s.to_wkt()?,
        label: s
            .authority()
            .unwrap_or_else(|_| s.name().unwrap_or_else(|| "Custom CRS".into())),
        units: if s.is_geographic() {
            s.angular_units_name()
        } else {
            s.linear_units_name()
        },
        default_tolerance: read::default_tolerance(Some(&s)),
    })
}
pub fn layer_metadata(sources: &[Arc<SourceLayer>], prepared: &Prepared) -> Vec<LayerView> {
    sources
        .iter()
        .map(|s| {
            let p = prepared.layer(&s.id);
            LayerView {
                id: s.id.clone(),
                dataset: s.dataset.clone(),
                selector: s.selector.clone(),
                name: s.name.clone(),
                feature_count: s.layer.len(),
                kinds: s.kinds.clone(),
                source_crs: s.crs.clone(),
                warnings: s.warnings.clone(),
                extent: p.and_then(|p| p.extent),
                display_extent: p.and_then(|p| p.display_extent),
            }
        })
        .collect()
}
