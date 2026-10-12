//! Incremental validation metadata and sink errors.
use crate::{Error, RuleReport, RuleSpec, ViolationKind, Warning};
use serde::Serialize;
use std::collections::BTreeMap;

/// One rule's metadata and counts, without retained violation geometry.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct RuleSummary {
    /// Registered rule specification.
    pub spec: RuleSpec,
    /// Primary layer name.
    pub layer: String,
    /// Secondary layer name.
    pub other: Option<String>,
    /// Aggregated violation counts.
    pub counts: BTreeMap<ViolationKind, usize>,
    /// Total aggregated violations.
    pub total: usize,
}
impl From<&RuleReport> for RuleSummary {
    fn from(report: &RuleReport) -> Self {
        Self {
            spec: report.spec,
            layer: report.layer.clone(),
            other: report.other.clone(),
            counts: report.counts.clone(),
            total: report.violations.len(),
        }
    }
}
/// Final metadata available after successful sink exhaustion.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct ReportSummary {
    /// Input-coordinate tolerance.
    pub tolerance: f64,
    /// Typed input warnings.
    pub warnings: Vec<Warning>,
    /// Summaries in rule registration order.
    pub rules: Vec<RuleSummary>,
}
impl ReportSummary {
    /// Count all aggregated violations.
    pub fn violation_count(&self) -> usize {
        self.rules.iter().map(|r| r.total).sum()
    }
    /// Whether every rule passed.
    pub fn is_valid(&self) -> bool {
        self.violation_count() == 0
    }
}
/// An operational error or the original error returned by the consumer.
#[derive(Debug, thiserror::Error)]
pub enum SinkError<E> {
    /// Invalid input or geometry preparation failure.
    #[error(transparent)]
    Core(#[from] Error),
    /// Consumer failure, including cancellation. No further output is produced.
    #[error("validation consumer failed: {0}")]
    Consumer(E),
}
