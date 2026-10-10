use crate::{FeatureRef, GeometryKind, LayerId, Rule};
use geo::Geometry;
use serde::{Deserialize, Serialize};

/// Operational failures are distinct from topology violations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("tolerance must be finite and nonnegative, found {0}")]
    Tolerance(f64),
    #[error("unknown layer {0:?}")]
    UnknownLayer(LayerId),
    #[error("layer {layer}, FID {fid}: expected {expected:?}, found {found:?}")]
    GeometryKind {
        layer: String,
        fid: i64,
        expected: GeometryKind,
        found: GeometryKind,
    },
    #[error("layer {layer}, FID {fid}: coordinates must be finite")]
    NonFinite { layer: String, fid: i64 },
    #[error("layer {layer}: duplicate FID {fid}")]
    DuplicateFid { layer: String, fid: i64 },
    #[error("{rule} requires {expected} input layer(s)")]
    Inputs { rule: Rule, expected: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorType {
    Overlap,
    Outside,
    Dangle,
    PseudoNode,
    Crossing,
    TouchInterior,
    SelfOverlap,
    SelfCrossing,
    Gap,
    OuterBoundary,
    Multipart,
}

impl ErrorType {
    pub fn name(self) -> &'static str {
        match self {
            Self::Overlap => "overlap",
            Self::Outside => "outside",
            Self::Dangle => "dangle",
            Self::PseudoNode => "pseudo-node",
            Self::Crossing => "crossing",
            Self::TouchInterior => "touch-interior",
            Self::SelfOverlap => "self-overlap",
            Self::SelfCrossing => "self-crossing",
            Self::Gap => "gap",
            Self::OuterBoundary => "outer-boundary",
            Self::Multipart => "multipart",
        }
    }
}

/// A violation, including provenance. `part: None` means multiple parts contributed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TopologyError {
    pub rule: Rule,
    pub error_type: ErrorType,
    pub geometry: Geometry<f64>,
    pub origin: Option<FeatureRef>,
    pub destination: Option<FeatureRef>,
}
