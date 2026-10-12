use crate::{FeatureRef, Rule};
use geo::Geometry;
use serde::{Deserialize, Serialize};

/// Categories of reported topology violations, ordered deterministically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ViolationKind {
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
    /// A point, endpoint or line piece without required coverage.
    NotCovered,
    /// A polygon feature without a qualifying point feature.
    MissingPoint,
    /// A polygon or point feature failing a one-to-one assignment.
    PointCount,
    /// A strictly contained point or whole feature.
    Contained,
    /// Any prohibited contact, including isolated points.
    Intersection,
}

impl ViolationKind {
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
            Self::NotCovered => "not-covered",
            Self::MissingPoint => "missing-point",
            Self::PointCount => "point-count",
            Self::Contained => "contained",
            Self::Intersection => "intersection",
        }
    }
}

/// A topology violation, including provenance. `part: None` means multiple parts contributed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Violation {
    /// Rule that produced the violation.
    pub rule: Rule,
    /// Violation category, serialized as `error_type`.
    #[serde(rename = "error_type")]
    pub kind: ViolationKind,
    /// Violation geometry in planar input coordinate units.
    /// Aggregation may produce multi geometries and deduplicates coincident points.
    pub geometry: Geometry<f64>,
    /// Primary feature provenance, absent for layer-wide results such as gaps.
    pub origin: Option<FeatureRef>,
    /// Secondary feature provenance for pair violations, when applicable.
    pub destination: Option<FeatureRef>,
}

impl Violation {
    /// Create a violation with geometry and topology-scoped source references.
    #[must_use]
    pub fn new(
        rule: Rule,
        kind: ViolationKind,
        geometry: Geometry,
        origin: Option<FeatureRef>,
        destination: Option<FeatureRef>,
    ) -> Self {
        Self {
            rule,
            kind,
            geometry,
            origin,
            destination,
        }
    }
}
