#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod aggregate;
pub(crate) mod engine;
mod error;
mod merge;
mod model;
mod rule;
mod rules;
mod topology;
mod violation;

pub use error::Error;
/// Geometry types and algorithms, with Geo's serde and multithreading features.
pub use geo;
pub use merge::{explode_lines, merge_lines};
pub use model::{FeatureRef, GeometryKind, Layer, LayerId};
pub use rule::{ParseRuleError, Rule, RuleId, RuleSpec};
pub use topology::{LayerInput, Report, RuleReport, Topology, Warning};
pub use violation::{Violation, ViolationKind};
