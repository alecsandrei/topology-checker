#![doc = include_str!("../README.md")]

pub(crate) mod engine;
mod error;
mod merge;
mod model;
mod rule;
mod rules;
mod topology;

pub use error::{Error, ErrorType, TopologyError};
pub use geo;
pub use merge::{explode_lines, merge_lines};
pub use model::{FeatureRef, GeometryKind, Layer, LayerId};
pub use rule::{Rule, RuleSpec};
pub use topology::{Report, RuleReport, Topology};
