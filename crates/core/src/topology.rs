use crate::engine::line_graph::{InputMode, LineGraph};
use crate::{
    Error, Layer, LayerId, Rule, RuleId, RuleSpec, Violation, ViolationKind, model::Parts, rules,
};
use rayon::prelude::*;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex, OnceLock},
};
type GraphSlot = Arc<OnceLock<Result<Arc<LineGraph>, Error>>>;

/// One rule's violations, with deterministic counts and feature provenance.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct RuleReport {
    /// Registered rule with its topology-scoped rule and layer IDs.
    pub spec: RuleSpec,
    /// Primary input layer name.
    pub layer: String,
    /// Secondary input layer name for a binary rule.
    pub other: Option<String>,
    /// Number of aggregated violations per kind, in kind order.
    pub counts: BTreeMap<ViolationKind, usize>,
    /// Deterministically ordered violations, serialized as `errors`.
    /// Several parts may contribute to one violation.
    #[serde(rename = "errors")]
    pub violations: Vec<Violation>,
}
impl RuleReport {
    /// Return the name of one of this rule's input layers, as referenced by its violations.
    #[must_use]
    pub fn layer_name(&self, id: LayerId) -> Option<&str> {
        if id == self.spec.layer {
            Some(&self.layer)
        } else if Some(id) == self.spec.other {
            self.other.as_deref()
        } else {
            None
        }
    }
}
/// A non-fatal input condition found while preparing a registered layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Warning {
    /// Degenerate parts or rings were skipped while cleaning a layer.
    DegenerateParts {
        /// Topology-scoped layer ID.
        layer: LayerId,
        /// Caller-supplied layer name, which need not be unique.
        name: String,
        /// Number of skipped parts and rings.
        skipped: usize,
    },
}
impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DegenerateParts { name, skipped, .. } => {
                write!(
                    f,
                    "layer {name}: skipped {skipped} degenerate parts or rings"
                )
            }
        }
    }
}
/// A successful validation result, which may contain topology violations.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct Report {
    /// Planar tolerance in input coordinate units.
    pub tolerance: f64,
    /// Degenerate-part warnings in layer registration order.
    pub warnings: Vec<Warning>,
    /// Rule results in rule registration order, independent of parallel scheduling.
    /// The result for `RuleId` `i` is at index `i`.
    pub rules: Vec<RuleReport>,
}
impl Report {
    /// Return whether every registered rule has no violations.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.rules.iter().all(|r| r.violations.is_empty())
    }
    /// Count aggregated violations across all registered rules.
    #[must_use]
    pub fn violation_count(&self) -> usize {
        self.rules.iter().map(|r| r.violations.len()).sum()
    }
    /// Iterate violations in rule order and then each rule's deterministic order.
    pub fn violations(&self) -> impl Iterator<Item = &Violation> {
        self.rules.iter().flat_map(|r| &r.violations)
    }
    /// Return one rule's result. IDs must originate from the topology that
    /// produced this report.
    #[must_use]
    pub fn rule(&self, id: RuleId) -> Option<&RuleReport> {
        self.rules.get(id.0)
    }
}
/// An existing layer identity or an owned layer to register atomically with a rule.
#[derive(Debug)]
pub enum LayerInput {
    /// Reuse a layer registered in this topology.
    Existing(LayerId),
    /// Register a fresh owned snapshot.
    New(Box<Layer>),
}

/// Shared planar layers and ordered rules with reusable, thread-safe geometry caches.
#[derive(Debug)]
pub struct Topology {
    tolerance: f64,
    layers: Vec<Arc<Layer>>,
    specs: Vec<RuleSpec>,
    graphs: Mutex<BTreeMap<Vec<(LayerId, InputMode)>, GraphSlot>>,
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
    /// Register a layer without validating it, returning a topology-scoped ID.
    /// Pass an `Arc<Layer>` to share one layer, and its cached views, between topologies.
    pub fn add_layer(&mut self, layer: impl Into<Arc<Layer>>) -> LayerId {
        let id = LayerId(self.layers.len());
        self.layers.push(layer.into());
        id
    }
    /// Borrow a registered layer. IDs must originate from this topology.
    ///
    /// # Errors
    /// Returns [`Error::UnknownLayer`] when the index is not registered.
    pub fn layer(&self, id: LayerId) -> Result<&Layer, Error> {
        self.layers
            .get(id.0)
            .map(AsRef::as_ref)
            .ok_or(Error::UnknownLayer(id))
    }
    /// Iterate layers and their IDs in registration order.
    pub fn layers(&self) -> impl Iterator<Item = (LayerId, &Layer)> {
        self.layers
            .iter()
            .enumerate()
            .map(|(i, l)| (LayerId(i), l.as_ref()))
    }
    /// Iterate registered rules in registration order.
    pub fn rules(&self) -> impl ExactSizeIterator<Item = RuleSpec> {
        self.specs.iter().copied()
    }
    /// Remove every registered rule, keeping layers and their cached line graphs, so a
    /// different rule set can be validated without rebuilding them. Rule IDs restart at
    /// zero; IDs from earlier registrations must not be reused.
    pub fn clear_rules(&mut self) {
        self.specs.clear();
    }
    pub(crate) fn parts(&self, id: LayerId) -> Result<&Parts, Error> {
        let layer = self.layer(id)?;
        layer.parts().map_err(|e| e.at(id, layer.name()))
    }
    pub(crate) fn graph(&self, layers: Vec<LayerId>) -> Result<Arc<LineGraph>, Error> {
        self.graph_inputs(
            layers
                .into_iter()
                .map(|id| (id, InputMode::Linework))
                .collect(),
        )
    }
    pub(crate) fn graph_inputs(
        &self,
        mut layers: Vec<(LayerId, InputMode)>,
    ) -> Result<Arc<LineGraph>, Error> {
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
    /// The returned spec's `id` locates this rule's result with [`Report::rule`].
    ///
    /// # Errors
    /// Returns input-count, same-layer, unknown-layer, geometry-kind, duplicate-FID
    /// or non-finite-coordinate errors. Failed registration does not add a rule.
    pub fn add_rule(
        &mut self,
        rule: Rule,
        layer: LayerId,
        other: Option<LayerId>,
    ) -> Result<RuleSpec, Error> {
        self.register_rule(
            rule,
            LayerInput::Existing(layer),
            other.map(LayerInput::Existing),
        )
    }
    /// Validate and commit owned inputs and a rule as one transaction.
    /// Layer caches do not depend on registration IDs, so validation work done
    /// here is reused after commit.
    ///
    /// # Errors
    /// Returns arity, same-layer, unknown-layer, geometry-kind, duplicate-FID or
    /// coordinate errors before committing any new layer or rule.
    pub fn register_rule(
        &mut self,
        rule: Rule,
        layer: LayerInput,
        other: Option<LayerInput>,
    ) -> Result<RuleSpec, Error> {
        let (primary_kind, other_kind) = rule.inputs();
        rule.check_input_count(other.is_some())?;
        if let (LayerInput::Existing(a), Some(LayerInput::Existing(b))) = (&layer, &other)
            && a == b
        {
            return Err(Error::SameLayer { rule, layer: *a });
        }
        let mut next = self.layers.len();
        let mut check = |input: &LayerInput, kind| -> Result<LayerId, Error> {
            let (id, value) = match input {
                LayerInput::Existing(id) => (*id, self.layer(*id)?),
                LayerInput::New(value) => {
                    let id = LayerId(next);
                    next += 1;
                    (id, value.as_ref())
                }
            };
            value.check_kind(id, kind)?;
            value.parts().map_err(|e| e.at(id, value.name()))?;
            Ok(id)
        };
        let primary_id = check(&layer, primary_kind)?;
        let other_id = other
            .as_ref()
            .zip(other_kind)
            .map(|(input, kind)| check(input, kind))
            .transpose()?;
        for input in [Some(layer), other].into_iter().flatten() {
            if let LayerInput::New(value) = input {
                self.layers.push(Arc::from(value));
            }
        }
        let spec = RuleSpec {
            id: RuleId(self.specs.len()),
            rule,
            layer: primary_id,
            other: other_id,
        };
        self.specs.push(spec);
        Ok(spec)
    }
    /// Validate cached part views and report skipped degenerate parts without running rules.
    ///
    /// # Errors
    /// Returns duplicate-FID or non-finite-coordinate errors from any registered layer,
    /// including layers that are not used by a rule.
    pub fn warnings(&self) -> Result<Vec<Warning>, Error> {
        let mut warnings = Vec::new();
        for (id, layer) in self.layers() {
            let skipped = self.parts(id)?.skipped;
            if skipped > 0 {
                warnings.push(Warning::DegenerateParts {
                    layer: id,
                    name: layer.name().into(),
                    skipped,
                });
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
                let violations = rules::run(self, spec)?;
                let mut counts = BTreeMap::new();
                for v in &violations {
                    *counts.entry(v.kind).or_default() += 1;
                }
                Ok(RuleReport {
                    spec,
                    layer: self.layer(spec.layer)?.name().into(),
                    other: spec
                        .other
                        .map(|id| self.layer(id).map(|l| l.name().to_string()))
                        .transpose()?,
                    counts,
                    violations,
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

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{LineString, Point};

    #[test]
    fn transactions_reject_invalid_layers_without_consuming_ids_or_rules() {
        for bad in [
            Layer::from_features("duplicates", vec![(4, Point::new(0., 0.).into()); 2]),
            Layer::from_geometries("nonfinite", [Point::new(f64::INFINITY, 0.).into()]),
            Layer::from_geometries(
                "wrong kind",
                [LineString::from(vec![(0., 0.), (1., 0.)]).into()],
            ),
        ] {
            let mut topology = Topology::new(0.).unwrap();
            let original = topology.add_layer(Layer::from_geometries("original", []));
            let new = LayerInput::New(Box::new(Layer::from_geometries(
                "pending",
                [Point::new(0., 0.).into()],
            )));
            assert!(
                topology
                    .register_rule(
                        Rule::PointMustNotOverlapWith,
                        new,
                        Some(LayerInput::New(Box::new(bad))),
                    )
                    .is_err()
            );
            assert_eq!(topology.layers().count(), 1);
            assert!(topology.validate().unwrap().rules.is_empty());
            assert!(topology.warnings().unwrap().is_empty());
            let spec = topology
                .register_rule(
                    Rule::PointMustNotOverlapWith,
                    LayerInput::Existing(original),
                    Some(LayerInput::New(Box::new(Layer::from_geometries(
                        "next",
                        [],
                    )))),
                )
                .unwrap();
            assert_eq!(spec.other.unwrap().index(), 1);
        }
    }

    #[test]
    fn transaction_caches_keep_final_ids_for_each_input_order() {
        for reuse_primary in [false, true] {
            let mut topology = Topology::new(0.).unwrap();
            topology.add_layer(Layer::from_geometries("unused", []));
            let existing = topology.add_layer(Layer::from_geometries(
                "existing",
                [Point::new(0., 0.).into()],
            ));
            let fresh = LayerInput::New(Box::new(Layer::from_geometries(
                "fresh",
                [Point::new(0., 0.).into()],
            )));
            let (primary, other) = if reuse_primary {
                (LayerInput::Existing(existing), fresh)
            } else {
                (fresh, LayerInput::Existing(existing))
            };
            let spec = topology
                .register_rule(Rule::PointMustNotOverlapWith, primary, Some(other))
                .unwrap();
            let report = topology.validate().unwrap();
            let error = &report.rules[0].violations[0];
            assert_eq!(error.origin.unwrap().layer, spec.layer);
            assert_eq!(error.destination.unwrap().layer, spec.other.unwrap());
            assert_eq!(topology.layers().count(), 3);
        }
    }
}
