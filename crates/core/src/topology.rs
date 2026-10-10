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

/// One rule's violations, with deterministic counts and feature provenance.
#[derive(Debug, Clone, Serialize)]
pub struct RuleReport {
    /// Registered rule and topology-scoped layer IDs.
    pub spec: RuleSpec,
    /// Primary input layer name.
    pub layer: String,
    /// Secondary input layer name for a binary rule.
    pub other: Option<String>,
    /// Number of aggregated violations per error type, in error-type order.
    pub counts: BTreeMap<ErrorType, usize>,
    /// Deterministically ordered violations; several parts may contribute to one.
    pub errors: Vec<TopologyError>,
}
/// A successful validation result, which may contain topology violations.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Planar tolerance in input coordinate units.
    pub tolerance: f64,
    /// Degenerate-part warnings in layer registration order.
    pub warnings: Vec<String>,
    /// Rule results in rule registration order, independent of parallel scheduling.
    pub rules: Vec<RuleReport>,
}
impl Report {
    /// Return whether every registered rule has no violations.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.rules.iter().all(|r| r.errors.is_empty())
    }
    /// Count aggregated violations across all registered rules.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.rules.iter().map(|r| r.errors.len()).sum()
    }
    /// Iterate violations in rule order and then each rule's deterministic order.
    pub fn errors(&self) -> impl Iterator<Item = &TopologyError> {
        self.rules.iter().flat_map(|r| &r.errors)
    }
}
/// Owned planar layers and ordered rules with reusable, thread-safe geometry caches.
#[derive(Debug)]
pub struct Topology {
    tolerance: f64,
    layers: Vec<Layer>,
    specs: Vec<RuleSpec>,
    graphs: Mutex<BTreeMap<Vec<LayerId>, GraphSlot>>,
}
impl Topology {
    /// Create an empty topology with tolerance in input coordinate units.
    /// Zero requests exact comparisons; no CRS transformation is performed.
    ///
    /// # Errors
    /// Returns [`Error::Tolerance`] for a negative or non-finite tolerance.
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
    /// Return the configured planar tolerance.
    #[must_use]
    pub fn tolerance(&self) -> f64 {
        self.tolerance
    }
    /// Register an owned layer without validating it, returning a topology-scoped ID.
    pub fn add_layer(&mut self, layer: Layer) -> LayerId {
        let id = LayerId(self.layers.len());
        self.layers.push(layer);
        id
    }
    /// Borrow a registered layer. IDs must originate from this topology.
    ///
    /// # Errors
    /// Returns [`Error::UnknownLayer`] when the index is not registered.
    pub fn layer(&self, id: LayerId) -> Result<&Layer, Error> {
        self.layers.get(id.0).ok_or(Error::UnknownLayer(id))
    }
    /// Iterate layers and their IDs in registration order.
    pub fn layers(&self) -> impl Iterator<Item = (LayerId, &Layer)> {
        self.layers.iter().enumerate().map(|(i, l)| (LayerId(i), l))
    }
    pub(crate) fn parts(&self, id: LayerId) -> Result<&Parts, Error> {
        self.layer(id)?.parts(id)
    }
    pub(crate) fn graph(&self, mut layers: Vec<LayerId>) -> Result<Arc<LineGraph>, Error> {
        layers.sort_unstable();
        layers.dedup();
        // A poisoned map still contains valid Arc/OnceLock entries: initialization
        // happens outside this lock, and no partially built graph is stored in it.
        let slot = Arc::clone(
            self.graphs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entry(layers.clone())
                .or_default(),
        );
        slot.get_or_init(|| LineGraph::build(self, &layers).map(Arc::new))
            .as_ref()
            .map(Arc::clone)
            .map_err(Clone::clone)
    }
    /// Register a rule after validating input families, finite coordinates and FIDs.
    /// Unary rules require no secondary layer; binary rules require a distinct one.
    /// Rules run in parallel, but reports retain registration order.
    ///
    /// # Errors
    /// Returns input-count, unknown-layer, geometry-kind, duplicate-FID or
    /// non-finite-coordinate errors. Failed registration does not add a rule.
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
    ///
    /// # Errors
    /// Returns duplicate-FID or non-finite-coordinate errors from any registered layer,
    /// including layers that are not used by a rule.
    pub fn warnings(&self) -> Result<Vec<String>, Error> {
        let mut warnings = Vec::new();
        for (id, layer) in self.layers() {
            let n = self.parts(id)?.skipped;
            if n > 0 {
                warnings.push(format!(
                    "layer {}: skipped {n} degenerate parts or rings",
                    layer.name()
                ));
            }
        }
        Ok(warnings)
    }
    /// Validate all layers and execute registered rules, reusing cached views.
    /// Topology violations are a successful report, not operational errors.
    ///
    /// # Errors
    /// Returns invalid-layer errors from deferred validation or operational errors
    /// encountered while building rule inputs. No partial report is returned.
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
            let mut group = group.into_iter();
            let mut first = group.next().expect("aggregation groups are nonempty");
            // Keep an unsupported first geometry as the fallback, as before.
            let geometry = std::mem::replace(
                &mut first.geometry,
                Geometry::MultiPoint(MultiPoint(Vec::new())),
            );
            let mut points = Vec::new();
            let mut lines = Vec::new();
            let mut polygons = Vec::new();
            let mut add_geometry = |geometry| {
                match geometry {
                    Geometry::Point(g) => points.push(g),
                    Geometry::MultiPoint(g) if !g.0.is_empty() => points.extend(g.0),
                    Geometry::Line(g) => lines.push(g.into()),
                    Geometry::LineString(g) => lines.push(g),
                    Geometry::MultiLineString(g) if !g.0.is_empty() => lines.extend(g.0),
                    Geometry::Polygon(g) => polygons.push(g),
                    Geometry::MultiPolygon(g) if !g.0.is_empty() => polygons.extend(g.0),
                    other => return Some(other),
                }
                None
            };
            if let Some(fallback) = add_geometry(geometry) {
                first.geometry = fallback;
            }
            for e in group {
                if let (Some(a), Some(b)) = (&mut first.origin, e.origin) {
                    a.aggregate(b);
                }
                if let (Some(a), Some(b)) = (&mut first.destination, e.destination) {
                    a.aggregate(b);
                }
                // Unsupported later geometries never replace the first fallback.
                let _ = add_geometry(e.geometry);
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

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{LineString, Point, Rect, coord};

    #[test]
    fn aggregation_retains_provenance_deduplication_and_geometry_priority() {
        let origin = |part| {
            Some(FeatureRef {
                layer: LayerId(0),
                fid: -4,
                part: Some(part),
            })
        };
        let error = |geometry, part| TopologyError {
            rule: Rule::LineMustNotIntersect,
            error_type: ErrorType::Crossing,
            geometry,
            origin: origin(part),
            destination: Some(FeatureRef {
                layer: LayerId(1),
                fid: 8,
                part: Some(2),
            }),
        };
        let point = Point::new(1., 0.);
        let points = aggregate(vec![error(point.into(), 0), error(point.into(), 1)]);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].geometry, Geometry::Point(point));
        assert_eq!(points[0].origin.unwrap().part, None);
        assert_eq!(points[0].destination.unwrap().part, Some(2));
        let line = LineString::from(vec![(0., 0.), (1., 0.)]);
        let polygon = Rect::new(coord! {x:0.,y:0.}, coord! {x:1.,y:1.}).to_polygon();
        let mixed = aggregate(vec![
            error(point.into(), 0),
            error(line.into(), 0),
            error(polygon.clone().into(), 1),
        ]);
        assert_eq!(mixed[0].geometry, Geometry::Polygon(polygon));
        let empty = Geometry::MultiLineString(MultiLineString(Vec::new()));
        assert_eq!(aggregate(vec![error(empty.clone(), 0)])[0].geometry, empty);
    }
}
