#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub(crate) mod engine;
mod error;
mod merge;
mod model;
mod rule;
mod rules;
mod topology;

pub use error::{Error, ErrorType, TopologyError};
/// Geometry types and algorithms, with Geo's serde and multithreading features.
pub use geo;
pub use merge::{explode_lines, merge_lines};
pub use model::{FeatureRef, GeometryKind, Layer, LayerId};
pub use rule::{ParseRuleError, Rule, RuleSpec};
pub use topology::{Report, RuleReport, Topology};
