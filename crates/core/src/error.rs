use crate::{GeometryKind, LayerId, Rule};

/// Operational failures are distinct from topology violations.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A negative or non-finite planar tolerance.
    #[error("tolerance must be finite and nonnegative, found {0}")]
    Tolerance(f64),
    /// A layer index not registered in the topology.
    #[error("unknown layer {0:?}")]
    UnknownLayer(LayerId),
    /// A feature incompatible with a rule's required geometry family.
    #[error("layer {name}, FID {fid}: expected {expected:?}, found {found:?}")]
    GeometryKind {
        /// Topology-scoped layer ID.
        layer: LayerId,
        /// Caller-supplied layer name, which need not be unique.
        name: String,
        /// Signed input feature ID.
        fid: i64,
        /// Required geometry family.
        expected: GeometryKind,
        /// Actual geometry family.
        found: GeometryKind,
    },
    /// An input feature contains an infinite or NaN coordinate.
    #[error("layer {name}, FID {fid}: coordinates must be finite")]
    NonFinite {
        /// Topology-scoped layer ID.
        layer: LayerId,
        /// Caller-supplied layer name, which need not be unique.
        name: String,
        /// Signed input feature ID.
        fid: i64,
    },
    /// Two features within one layer share an ID.
    #[error("layer {name}: duplicate FID {fid}")]
    DuplicateFid {
        /// Topology-scoped layer ID.
        layer: LayerId,
        /// Caller-supplied layer name, which need not be unique.
        name: String,
        /// Repeated signed feature ID.
        fid: i64,
    },
    /// An incorrect number of input layers for a rule.
    #[error("{rule} requires {expected} input layer(s)")]
    Inputs {
        /// Rule being registered.
        rule: Rule,
        /// Required number of distinct input layers.
        expected: usize,
    },
    /// One layer supplied as both inputs of a binary rule.
    #[error("{rule} requires two distinct input layers")]
    SameLayer {
        /// Rule being registered.
        rule: Rule,
        /// Layer supplied twice.
        layer: LayerId,
    },
    /// A line utility input contains an infinite or NaN coordinate.
    #[error("line {index}: coordinates must be finite")]
    NonFiniteLine {
        /// Zero-based position in the input lines.
        index: usize,
    },
}
