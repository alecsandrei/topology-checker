use crate::engine::line_graph::LineGraph;
use crate::{
    Error, ErrorType, FeatureRef, Layer, LayerId, Rule, RuleSpec, TopologyError, model::Parts,
    rules,
};
use geo::{Geometry, MultiLineString, MultiPoint, MultiPolygon};
use rayon::prelude::*;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};
type GraphSlot = Arc<OnceLock<Result<Arc<LineGraph>, Error>>>;

#[derive(Debug, Clone, Serialize)]
pub struct RuleReport {
    pub spec: RuleSpec,
    pub layer: String,
    pub other: Option<String>,
    pub counts: BTreeMap<ErrorType, usize>,
    pub errors: Vec<TopologyError>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub tolerance: f64,
    pub warnings: Vec<String>,
    pub rules: Vec<RuleReport>,
}
impl Report {
    pub fn is_valid(&self) -> bool {
        self.rules.iter().all(|r| r.errors.is_empty())
    }
    pub fn error_count(&self) -> usize {
        self.rules.iter().map(|r| r.errors.len()).sum()
    }
    pub fn errors(&self) -> impl Iterator<Item = &TopologyError> {
        self.rules.iter().flat_map(|r| &r.errors)
    }
}
#[derive(Debug)]
pub struct Topology {
    tolerance: f64,
    layers: Vec<Layer>,
    specs: Vec<RuleSpec>,
    graphs: Mutex<BTreeMap<Vec<LayerId>, GraphSlot>>,
}
impl Topology {
    pub fn new(tolerance: f64) -> Result<Self, Error> {
        if !tolerance.is_finite() || tolerance < 0. {
            return Err(Error::Tolerance(tolerance));
        }
        Ok(Self {
            tolerance,
            layers: Vec::new(),
            specs: Vec::new(),
            graphs: Mutex::new(BTreeMap::new()),
        })
    }
    pub fn tolerance(&self) -> f64 {
        self.tolerance
    }
    pub fn add_layer(&mut self, layer: Layer) -> LayerId {
        let id = LayerId(self.layers.len());
        self.layers.push(layer);
        id
    }
    pub fn layer(&self, id: LayerId) -> Result<&Layer, Error> {
        self.layers.get(id.0).ok_or(Error::UnknownLayer(id))
    }
    pub fn layers(&self) -> impl Iterator<Item = (LayerId, &Layer)> {
        self.layers.iter().enumerate().map(|(i, l)| (LayerId(i), l))
    }
    pub(crate) fn parts(&self, id: LayerId) -> Result<&Parts, Error> {
        self.layer(id)?.parts(id)
    }
    pub(crate) fn graph(&self, mut layers: Vec<LayerId>) -> Result<Arc<LineGraph>, Error> {
        layers.sort_unstable();
        layers.dedup();
        let slot = self
            .graphs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(layers.clone())
            .or_default()
            .clone();
        slot.get_or_init(|| LineGraph::build(self, &layers).map(Arc::new))
            .clone()
    }
    pub fn add_rule(
        &mut self,
        rule: Rule,
        layer: LayerId,
        other: Option<LayerId>,
    ) -> Result<(), Error> {
        let (a, b) = rule.inputs();
        if other.is_some() != b.is_some() || other == Some(layer) {
            return Err(Error::Inputs {
                rule,
                expected: if b.is_some() { 2 } else { 1 },
            });
        }
        self.layer(layer)?.check_kind(a)?;
        self.parts(layer)?;
        if let (Some(id), Some(kind)) = (other, b) {
            self.layer(id)?.check_kind(kind)?;
            self.parts(id)?;
        }
        self.specs.push(RuleSpec { rule, layer, other });
        Ok(())
    }
    /// Validate cached part views and report skipped degenerate parts without running rules.
    pub fn warnings(&self) -> Result<Vec<String>, Error> {
        Ok(self
            .layers()
            .map(|(id, layer)| self.parts(id).map(|p| (layer, p.skipped)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .map(|(l, n)| format!("layer {}: skipped {n} degenerate parts or rings", l.name()))
            .collect())
    }
    pub fn validate(&self) -> Result<Report, Error> {
        let warnings = self.warnings()?;
        let rules = self
            .specs
            .par_iter()
            .map(|&spec| {
                let errors = rules::run(self, spec)?;
                let mut counts = BTreeMap::new();
                for e in &errors {
                    *counts.entry(e.error_type).or_default() += 1;
                }
                Ok(RuleReport {
                    spec,
                    layer: self.layer(spec.layer)?.name().into(),
                    other: spec
                        .other
                        .map(|id| self.layer(id).map(|l| l.name().to_string()))
                        .transpose()?,
                    counts,
                    errors,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Report {
            tolerance: self.tolerance,
            warnings,
            rules,
        })
    }
}

/// Aggregate pair errors across parts without inventing a single part identity.
pub(crate) fn aggregate(errors: Vec<TopologyError>) -> Vec<TopologyError> {
    type Key = (
        Rule,
        ErrorType,
        Option<(LayerId, i64)>,
        Option<(LayerId, i64)>,
    );
    let mut groups: BTreeMap<Key, Vec<TopologyError>> = BTreeMap::new();
    for e in errors {
        groups
            .entry((
                e.rule,
                e.error_type,
                e.origin.map(FeatureRef::feature),
                e.destination.map(FeatureRef::feature),
            ))
            .or_default()
            .push(e);
    }
    groups
        .into_values()
        .map(|group| {
            let mut first = group[0].clone();
            let mut points = Vec::new();
            let mut lines = Vec::new();
            let mut polygons = Vec::new();
            for e in &group {
                if let (Some(a), Some(b)) = (&mut first.origin, e.origin) {
                    a.aggregate(b);
                }
                if let (Some(a), Some(b)) = (&mut first.destination, e.destination) {
                    a.aggregate(b);
                }
                match &e.geometry {
                    Geometry::Point(g) => points.push(*g),
                    Geometry::MultiPoint(g) => points.extend(&g.0),
                    Geometry::Line(g) => lines.push((*g).into()),
                    Geometry::LineString(g) => lines.push(g.clone()),
                    Geometry::MultiLineString(g) => lines.extend(g.0.iter().cloned()),
                    Geometry::Polygon(g) => polygons.push(g.clone()),
                    Geometry::MultiPolygon(g) => polygons.extend(g.0.iter().cloned()),
                    _ => {}
                }
            }
            if !points.is_empty() {
                points.sort_by(|a, b| a.x().total_cmp(&b.x()).then(a.y().total_cmp(&b.y())));
                points.dedup();
                first.geometry = if points.len() == 1 {
                    Geometry::Point(points[0])
                } else {
                    Geometry::MultiPoint(MultiPoint(points))
                };
            }
            if !lines.is_empty() {
                first.geometry = if lines.len() == 1 {
                    Geometry::LineString(lines.remove(0))
                } else {
                    Geometry::MultiLineString(MultiLineString(lines))
                };
            }
            if !polygons.is_empty() {
                first.geometry = if polygons.len() == 1 {
                    Geometry::Polygon(polygons.remove(0))
                } else {
                    Geometry::MultiPolygon(MultiPolygon(polygons))
                };
            }
            first
        })
        .collect()
}
