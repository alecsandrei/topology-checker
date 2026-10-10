use crate::{FeatureRef, GeometryKind, LayerId, Rule};
use geo::Geometry;
use serde::{Deserialize, Serialize};

/// Operational failures are distinct from topology violations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    /// A negative or non-finite planar tolerance.
    #[error("tolerance must be finite and nonnegative, found {0}")]
    Tolerance(f64),
    /// A layer index not registered in the topology.
    #[error("unknown layer {0:?}")]
    UnknownLayer(LayerId),
    /// A feature incompatible with a rule's required geometry family.
    #[error("layer {layer}, FID {fid}: expected {expected:?}, found {found:?}")]
    GeometryKind {
        /// Input layer name.
        layer: String,
        /// Signed input feature ID.
        fid: i64,
        /// Required geometry family.
        expected: GeometryKind,
        /// Actual geometry family.
        found: GeometryKind,
    },
    /// An input feature contains an infinite or NaN coordinate.
    #[error("layer {layer}, FID {fid}: coordinates must be finite")]
    NonFinite {
        /// Input layer name.
        layer: String,
        /// Signed input feature ID.
        fid: i64,
    },
    /// Two features within one layer share an ID.
    #[error("layer {layer}: duplicate FID {fid}")]
    DuplicateFid {
        /// Input layer name.
        layer: String,
        /// Repeated signed feature ID.
        fid: i64,
    },
    /// Incorrect input count or reuse of one layer for both binary inputs.
    #[error("{rule} requires {expected} input layer(s)")]
    Inputs {
        /// Rule being registered.
        rule: Rule,
        /// Required number of distinct input layers.
        expected: usize,
    },
}

/// Categories of reported topology violations, ordered deterministically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorType {
    /// Coincident points, shared linework or polygon intersection area.
    Overlap,
    /// Geometry outside the required polygon coverage.
    Outside,
    /// A node with one incident endpoint use.
    Dangle,
    /// A node joining endpoints of exactly two distinct features.
    PseudoNode,
    /// An interior intersection between distinct features.
    Crossing,
    /// A meeting with at least one feature interior.
    TouchInterior,
    /// Repeated linework within one feature.
    SelfOverlap,
    /// A true self-crossing within one feature.
    SelfCrossing,
    /// An interior ring of the polygon union.
    Gap,
    /// An exterior ring of the polygon union, reported by the gaps rule.
    OuterBoundary,
    /// A feature containing multiple parts.
    Multipart,
}

impl ErrorType {
    /// Return the stable kebab-case output name.
    #[must_use]
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
    /// Rule that produced the violation.
    pub rule: Rule,
    /// Violation category.
    pub error_type: ErrorType,
    /// Violation geometry in planar input coordinate units.
    /// Aggregation may produce multi geometries and deduplicates coincident points.
    pub geometry: Geometry<f64>,
    /// Primary feature provenance, absent for layer-wide results such as gaps.
    pub origin: Option<FeatureRef>,
    /// Secondary feature provenance for pair violations, when applicable.
    pub destination: Option<FeatureRef>,
}
