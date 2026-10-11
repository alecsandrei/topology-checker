use anyhow::{Context, Result, bail};
use gdal::spatial_ref::CoordTransform;
use geo::{BoundingRect, CoordsIter, Geometry};
use rstar::{AABB, RTree, RTreeObject};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use topology_checker_core::{GeometryKind, Layer, Report, Rule, Topology};
use topology_checker_io::{crs, read};

pub const FEATURE_LIMIT: usize = 10_000;
pub const COORDINATE_LIMIT: usize = 200_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerSettings {
    pub id: String,
    pub selected: bool,
    pub visible: bool,
    pub color: String,
    pub source_crs: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewSettings {
    pub basemap: bool,
    pub center: Option<[f64; 2]>,
    pub resolution: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetSource {
    pub id: String,
    pub filename: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
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
    pub features: Vec<(i64, Geometry)>,
    pub attributes: Arc<Vec<Map<String, Value>>>,
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
#[derive(Debug)]
pub struct IndexedLayer {
    pub source: Arc<SourceLayer>,
    pub features: Vec<(i64, Geometry)>,
    index: RTree<Entry>,
    ids: HashMap<i64, usize>,
    pub extent: Option<[f64; 4]>,
}
impl IndexedLayer {
    /// Index complete geometries by extent and feature identity.
    ///
    /// # Errors
    /// Returns an error for duplicate FIDs or non-finite coordinates.
    pub fn new(source: Arc<SourceLayer>, features: Vec<(i64, Geometry)>) -> Result<Self> {
        let mut entries = Vec::new();
        let mut extent: Option<[f64; 4]> = None;
        let mut ids = HashMap::new();
        for (i, (fid, g)) in features.iter().enumerate() {
            if ids.insert(*fid, i).is_some() {
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
        Ok(Self {
            source,
            features,
            index: RTree::bulk_load(entries),
            ids,
            extent,
        })
    }
    pub fn feature(&self, fid: i64) -> Option<Value> {
        self.ids
            .get(&fid)
            .map(|&i| self.json_feature(i, &self.features[i].1))
    }
    fn json_feature(&self, ordinal: usize, g: &Geometry) -> Value {
        json!({"type":"Feature", "id":self.features[ordinal].0.to_string(), "geometry":geometry_json(g), "properties":self.source.attributes.get(ordinal), "layer":self.source.id})
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
    ) -> Result<Value> {
        let to_working = if display {
            Some(CoordTransform::new(
                &crs::parse("EPSG:3857")?,
                &crs::parse(working_crs.context("assign a working CRS for map display")?)?,
            )?)
        } else {
            None
        };
        let bounds = if let Some(t) = &to_working {
            t.transform_bounds(&bounds, 21)?
        } else {
            bounds
        };
        let to_display = if display {
            Some(CoordTransform::new(
                &crs::parse(working_crs.unwrap())?,
                &crs::parse("EPSG:3857")?,
            )?)
        } else {
            None
        };
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
            let g = &self.features[entry.ordinal].1;
            let transformed;
            let g = if let Some(t) = &to_display {
                transformed = crs::transform(g, t)?;
                &transformed
            } else {
                g
            };
            output.push(self.json_feature(entry.ordinal, g));
            coordinates += entry.coordinates;
        }
        Ok(
            json!({"type":"FeatureCollection", "features":output,"incomplete":incomplete,"coordinates":coordinates,"feature_limit":FEATURE_LIMIT,"coordinate_limit":COORDINATE_LIMIT}),
        )
    }
}
pub fn geometry_json(g: &Geometry) -> Value {
    serde_json::to_value(geojson::Geometry::new(geojson::Value::from(g))).expect("finite geometry")
}
pub fn kinds(features: &[(i64, Geometry)]) -> Vec<GeometryKind> {
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
    for (_, g) in features {
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
        let (a, b) = c.rule.inputs();
        if c.other.is_some() != b.is_some() || c.other.as_ref() == Some(&c.layer) {
            bail!("{} has invalid inputs", c.rule);
        }
        for (id, kind) in [(Some(&c.layer), Some(a)), (c.other.as_ref(), b)] {
            if let (Some(id), Some(kind)) = (id, kind) {
                let s = sources
                    .iter()
                    .find(|s| &s.id == id)
                    .context("unknown rule layer")?;
                if !config.layers.iter().any(|l| &l.id == id && l.selected) {
                    bail!("rule input is not selected");
                }
                if kind != GeometryKind::Any && s.kinds.iter().any(|k| *k != kind) {
                    bail!("{} requires {kind:?} input", c.rule);
                }
            }
        }
    }
    Ok(())
}
/// Transform original layers into the working CRS and index their complete geometries.
///
/// # Errors
/// Returns an error for missing source CRSs, failed transforms or invalid geometries.
pub fn prepare(config: &Config, sources: &[Arc<SourceLayer>]) -> Result<Vec<Arc<IndexedLayer>>> {
    let target = config.working_crs.as_deref().map(crs::parse).transpose()?;
    sources
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
            let features = s
                .features
                .iter()
                .map(|(fid, g)| {
                    Ok((
                        *fid,
                        if let Some(t) = &transform {
                            crs::transform(g, t)
                                .with_context(|| format!("transforming {}, FID {fid}", s.name))?
                        } else {
                            g.clone()
                        },
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Arc::new(IndexedLayer::new(Arc::clone(s), features)?))
        })
        .collect()
}
#[derive(Debug)]
pub struct ValidationResult {
    pub revision: u64,
    pub config: Config,
    pub layers: Vec<Arc<IndexedLayer>>,
    pub report: Report,
    rows: Vec<(usize, usize)>,
}
impl ValidationResult {
    /// Validate an immutable workspace configuration with the existing core.
    ///
    /// # Errors
    /// Returns an error for invalid rule inputs or core validation failures.
    pub fn run(revision: u64, config: Config, layers: Vec<Arc<IndexedLayer>>) -> Result<Self> {
        let mut topology = Topology::new(config.tolerance)?;
        let mut ids = HashMap::new();
        for l in &layers {
            ids.insert(
                &l.source.id,
                topology.add_layer(Layer::from_features(&l.source.name, l.features.clone())),
            );
        }
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
        if !config.checks.iter().any(|c| c.enabled) {
            bail!("enable at least one rule");
        }
        let mut report = topology.validate()?;
        for l in &layers {
            report.warnings.extend(l.source.warnings.clone());
        }
        if config.working_crs.is_none() {
            report.warnings.push(
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
            report
                .warnings
                .push("Tolerance is in angular units; calculations are planar".into());
        }
        let rows = report
            .rules
            .iter()
            .enumerate()
            .flat_map(|(r, rule)| (0..rule.errors.len()).map(move |e| (r, e)))
            .collect();
        Ok(Self {
            revision,
            config,
            layers,
            report,
            rows,
        })
    }
    fn reference(&self, r: Option<topology_checker_core::FeatureRef>) -> Value {
        r.map_or(Value::Null, |r| {
            let l = &self.layers[r.layer.index()];
            json!({"layer":l.source.id,"name":l.source.name,"dataset":l.source.dataset,"selector":l.source.selector,"fid":r.fid.to_string(),"part":r.part,"attributes":l.ids.get(&r.fid).and_then(|&i|l.source.attributes.get(i))})
        })
    }
    /// Serialize a single aggregated violation and signed source references.
    ///
    /// # Errors
    /// Returns an error for an unknown violation or failed display transformation.
    pub fn violation(&self, index: usize, display: bool) -> Result<Value> {
        let &(rule, error) = self.rows.get(index).context("unknown violation")?;
        let e = &self.report.rules[rule].errors[error];
        let transform = if display {
            Some(CoordTransform::new(
                &crs::parse(
                    self.config
                        .working_crs
                        .as_deref()
                        .context("working CRS is unknown")?,
                )?,
                &crs::parse("EPSG:3857")?,
            )?)
        } else {
            None
        };
        let g = if let Some(t) = &transform {
            crs::transform(&e.geometry, t)?
        } else {
            e.geometry.clone()
        };
        Ok(
            json!({"id":index,"rule":e.rule,"category":e.error_type,"geometry":geometry_json(&g),"origin":self.reference(e.origin),"destination":self.reference(e.destination)}),
        )
    }
    pub fn summary(&self) -> Value {
        json!({"revision":self.revision,"config":self.config,"valid":self.report.is_valid(),"total":self.report.error_count(),"warnings":self.report.warnings,"rules":self.report.rules.iter().map(|r| json!({"rule":r.spec.rule,"layer":self.layers[r.spec.layer.index()].source.id,"other":r.spec.other.map(|id| &self.layers[id.index()].source.id),"counts":r.counts,"total":r.errors.len()})).collect::<Vec<_>>()})
    }
    /// Return up to 100 violations in deterministic core report order.
    ///
    /// # Errors
    /// Returns an error for page offset overflow or violation serialization failures.
    pub fn page(&self, page: usize) -> Result<Value> {
        let start = page.checked_mul(100).context("page out of range")?;
        let rows = (start..(start.saturating_add(100)).min(self.report.error_count()))
            .map(|i| self.violation(i, false))
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({"page":page,"page_size":100,"total":self.report.error_count(),"violations":rows}))
    }
    /// Serialize the complete report with its configuration snapshot.
    ///
    /// # Errors
    /// Returns an error if a violation cannot be serialized.
    pub fn complete_json(&self) -> Result<Value> {
        let mut summary = self.summary();
        summary["violations"] = Value::Array(
            (0..self.report.error_count())
                .map(|i| self.violation(i, false))
                .collect::<Result<_>>()?,
        );
        Ok(summary)
    }
}
pub fn catalog() -> Value {
    json!(Rule::ALL.iter().map(|r| { let (primary,secondary) = r.inputs(); json!({"name":r.name(),"description":r.description(),"primary":primary,"secondary":secondary}) }).collect::<Vec<_>>())
}
/// Describe a working CRS, its units and default tolerance.
///
/// # Errors
/// Returns an error for invalid CRS definitions or failed GDAL metadata conversion.
pub fn crs_info(def: &str) -> Result<Value> {
    let s = crs::parse(def)?;
    Ok(
        json!({"definition":s.to_wkt()?,"label":s.authority().unwrap_or_else(|_|s.name().unwrap_or_else(||"Custom CRS".into())),"units":if s.is_geographic(){s.angular_units_name()}else{s.linear_units_name()},"default_tolerance":read::default_tolerance(Some(&s))}),
    )
}
pub fn layer_metadata(
    sources: &[Arc<SourceLayer>],
    prepared: &[Arc<IndexedLayer>],
    config: &Config,
) -> Vec<Value> {
    sources.iter().map(|s| {
        let p = prepared.iter().find(|p| p.source.id == s.id);
        let extent = p.and_then(|p| p.extent);
        let display_extent = extent.and_then(|e| config.working_crs.as_deref().and_then(|def| { let from = crs::parse(def).ok()?; let to = crs::parse("EPSG:3857").ok()?; CoordTransform::new(&from,&to).ok()?.transform_bounds(&e,21).ok() }));
        json!({"id":s.id,"dataset":s.dataset,"selector":s.selector,"name":s.name,"feature_count":s.features.len(),"kinds":s.kinds,"source_crs":s.crs,"warnings":s.warnings,"extent":extent,"display_extent":display_extent})
    }).collect()
}
