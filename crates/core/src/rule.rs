use crate::{GeometryKind, LayerId};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

macro_rules! registry {
    ($(($variant:ident, $name:literal, $a:ident, $b:expr, $description:literal, $arcgis:expr, $qgis:expr)),* $(,)?) => {
        /// A supported topology rule with its canonical CLI and serialization name.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub enum Rule { $( #[doc = $description] #[serde(rename = $name)] $variant, )* }
        impl Rule {
            /// All 38 rules in registry order. The original 18 remain the prefix.
            pub const ALL: &'static [Self] = &[$(Self::$variant),*];
            /// Return the canonical slash-separated rule name.
            #[must_use]
            pub const fn name(self) -> &'static str { match self { $(Self::$variant => $name),* } }
            /// Return the primary family and optional secondary input family.
            #[must_use]
            pub const fn inputs(self) -> (GeometryKind, Option<GeometryKind>) { match self { $(Self::$variant => (GeometryKind::$a, $b)),* } }
            /// Describe the rule's violation semantics and tolerance behavior.
            #[must_use]
            pub const fn description(self) -> &'static str { match self { $(Self::$variant => $description),* } }
            /// Return the corresponding ArcGIS name, when available.
            #[must_use]
            pub const fn arcgis(self) -> Option<&'static str> { match self { $(Self::$variant => $arcgis),* } }
            /// Return the corresponding QGIS name, when available.
            #[must_use]
            pub const fn qgis(self) -> Option<&'static str> { match self { $(Self::$variant => $qgis),* } }
        }
    }
}
use GeometryKind::{Any, Line, Point, Polygon};
registry! {
    (PointMustNotOverlap, "point/must-not-overlap", Point, None, "Report each pair of coincident points within tolerance once.", Some("Must Be Disjoint"), Some("Must not have duplicates")),
    (PointMustNotOverlapWith, "point/must-not-overlap-with", Point, Some(Point), "Report coincident point pairs across two layers.", None, None),
    (PointMustBeInside, "point/must-be-inside", Point, Some(Polygon), "Report points not strictly inside any polygon; boundary distances at or below tolerance are outside.", Some("Must Be Properly Inside Polygons"), Some("Must be inside")),
    (LineMustNotHaveDangles, "line/must-not-have-dangles", Line, None, "Report part endpoints with one incident use after noding and clustering.", Some("Must Not Have Dangles"), Some("Must not have dangles")),
    (LineMustNotHavePseudoNodes, "line/must-not-have-pseudo-nodes", Line, None, "Report nodes with exactly two endpoint uses from distinct features. Closed loops are valid.", Some("Must Not Have Pseudo Nodes"), Some("Must not have pseudo nodes")),
    (LineMustNotOverlap, "line/must-not-overlap", Line, None, "Report shared linework between distinct features, once per feature pair. Self-overlaps are excluded.", Some("Must Not Overlap"), None),
    (LineMustNotOverlapWith, "line/must-not-overlap-with", Line, Some(Line), "Report shared linework across two layers, once per feature pair.", Some("Must Not Overlap With"), None),
    (LineMustNotSelfOverlap, "line/must-not-self-overlap", Line, None, "Report linework traversed repeatedly by one feature, including its multiple parts.", Some("Must Not Self-Overlap"), None),
    (LineMustNotIntersect, "line/must-not-intersect", Line, None, "Report overlaps and interior-to-interior intersections between distinct features. Endpoint touches and self violations are excluded.", Some("Must Not Intersect"), None),
    (LineMustNotIntersectWith, "line/must-not-intersect-with", Line, Some(Line), "Report overlaps and interior-to-interior intersections across two layers.", Some("Must Not Intersect With"), None),
    (LineMustNotIntersectOrTouchInterior, "line/must-not-intersect-or-touch-interior", Line, None, "Report overlaps and meetings between distinct features where at least one feature is interior.", Some("Must Not Intersect Or Touch Interior"), None),
    (LineMustNotIntersectOrTouchInteriorWith, "line/must-not-intersect-or-touch-interior-with", Line, Some(Line), "Report overlaps and meetings across two layers where at least one feature is interior.", Some("Must Not Intersect Or Touch Interior With"), None),
    (LineMustNotSelfIntersect, "line/must-not-self-intersect", Line, None, "Report self-overlaps and true self-crossings. Closed loops and self-touches are valid.", Some("Must Not Self-Intersect"), None),
    (LineMustBeInside, "line/must-be-inside", Line, Some(Polygon), "Report pieces outside the polygon union longer than tolerance. Polygon boundaries are covered.", Some("Must Be Inside"), None),
    (PolygonMustNotOverlap, "polygon/must-not-overlap", Polygon, None, "Report polygon intersection areas once per distinct feature pair, excluding slivers below tolerance.", Some("Must Not Overlap"), Some("Must not overlap")),
    (PolygonMustNotOverlapWith, "polygon/must-not-overlap-with", Polygon, Some(Polygon), "Report polygon intersection areas across two layers, excluding slivers below tolerance.", Some("Must Not Overlap With"), Some("Must not overlap with")),
    (PolygonMustNotHaveGaps, "polygon/must-not-have-gaps", Polygon, None, "Report union interior rings as gaps and exterior rings as outer-boundary. Ignore sliver holes below tolerance.", Some("Must Not Have Gaps"), Some("Must not have gaps")),
    (GeometryMustNotBeMultipart, "geometry/must-not-be-multipart", Any, None, "Report features containing more than one part. A single-part Multi geometry is valid.", Some("Must Be Single Part (lines)"), Some("Must not have multi-part geometries")),
    (PointMustBeCoveredByLines, "point/must-be-covered-by-lines", Point, Some(Line), "Every point must be within tolerance of linework, including endpoints.", Some("Must Be Covered By (Point-Line)"), Some("Must be covered by")),
    (PointMustBeCoveredByLineEndpoints, "point/must-be-covered-by-line-endpoints", Point, Some(Line), "Every point must be within tolerance of an open line part endpoint. Closed parts have no endpoints.", Some("Must Be Covered By Endpoint Of"), Some("Must be covered by endpoints of")),
    (PointMustBeCoveredByPolygonBoundaries, "point/must-be-covered-by-polygon-boundaries", Point, Some(Polygon), "Every point must be within tolerance of an individual polygon exterior or hole boundary, including shared edges.", Some("Must Be Covered By Boundary Of (Point-Area)"), None),
    (LineEndpointsMustBeCoveredByPoints, "line/endpoints-must-be-covered-by-points", Line, Some(Point), "Every open line part endpoint must be within tolerance of a point. Closed parts have no endpoints.", Some("Endpoint Must Be Covered By"), Some("End points must be covered by")),
    (LineMustBeCoveredByPolygonBoundaries, "line/must-be-covered-by-polygon-boundaries", Line, Some(Polygon), "Report line pieces not covered by individual polygon boundaries, longer than tolerance. Includes hole and shared boundaries.", Some("Must Be Covered By Boundary Of (Line-Area)"), None),
    (PolygonBoundariesMustBeCoveredByLines, "polygon/boundaries-must-be-covered-by-lines", Polygon, Some(Line), "Report exterior and hole boundary pieces without covering linework, longer than tolerance.", Some("Boundary Must Be Covered By (Area-Line)"), None),
    (PolygonMustContainPoint, "polygon/must-contain-point", Polygon, Some(Point), "Every polygon feature must strictly contain all retained parts of at least one point feature. Count distinct FIDs.", Some("Contains Point"), Some("Must contain")),
    (PolygonMustContainOnePoint, "polygon/must-contain-one-point", Polygon, Some(Point), "Every polygon feature must strictly contain exactly one point feature, and every point feature must belong to exactly one polygon feature. Count distinct FIDs.", Some("Contains One Point"), None),
    (LineMustNotCrossPolygonBoundaries, "line/must-not-cross-polygon-boundaries", Line, Some(Polygon), "Report entry and exit crossings of individual exterior and hole boundaries. Tangencies, endpoint contacts and shared segments are allowed.", None, None),
    (GeometryMustNotBeContainedBy, "geometry/must-not-be-contained-by", Any, Some(Any), "Report whole cleaned features topologically contained by individual target features, excluding mutual containment. No tolerance buffer is used.", None, Some("Check Geometry (Contained)")),
    (PointMustBeCoveredByPolygons, "point/must-be-covered-by-polygons", Point, Some(Polygon), "Every point must be inside polygon areas or within tolerance of their boundaries.", None, None),
    (PointMustNotBeInsidePolygons, "point/must-not-be-inside-polygons", Point, Some(Polygon), "Report strictly interior points. Points within tolerance of polygon boundaries are allowed.", None, None),
    (PointMustNotIntersectLines, "point/must-not-intersect-lines", Point, Some(Line), "Report point contacts within tolerance anywhere on linework, including endpoints.", None, None),
    (PointMustNotIntersectPolygons, "point/must-not-intersect-polygons", Point, Some(Polygon), "Report points inside polygon areas or within tolerance of their boundaries.", None, None),
    (LineMustBeProperlyInsidePolygons, "line/must-be-properly-inside-polygons", Line, Some(Polygon), "Every cleaned line part must be inside the polygon union and farther than tolerance from its boundary. Report the whole failing part.", None, None),
    (LineMustNotOverlapWithPolygons, "line/must-not-overlap-with-polygons", Line, Some(Polygon), "Report line pieces inside individual polygon areas or along their boundaries, longer than tolerance. Isolated contacts are allowed.", None, None),
    (LineMustNotOverlapPolygonBoundaries, "line/must-not-overlap-polygon-boundaries", Line, Some(Polygon), "Report shared individual polygon boundary segments longer than tolerance. Crossings and isolated contacts are allowed.", None, None),
    (LineMustNotIntersectPolygons, "line/must-not-intersect-polygons", Line, Some(Polygon), "Report interior pieces, shared boundary pieces and isolated contacts with individual polygons. Boundary proximity within tolerance counts as contact.", None, None),
    (LineEndpointsMustBeCoveredByPolygonBoundaries, "line/endpoints-must-be-covered-by-polygon-boundaries", Line, Some(Polygon), "Every open line part endpoint must be within tolerance of an exterior or hole boundary. Closed parts have no endpoints.", None, None),
    (PolygonMustNotContainPoints, "polygon/must-not-contain-points", Polygon, Some(Point), "Report strictly contained point parts with polygon origin provenance. Points within tolerance of boundaries are allowed.", None, None),
}
impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
impl FromStr for Rule {
    type Err = ParseRuleError;
    /// Parse an exact, case-sensitive canonical rule name.
    ///
    /// # Errors
    /// Returns [`ParseRuleError`] containing the unrecognized input name.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|r| r.name() == value)
            .ok_or_else(|| ParseRuleError { name: value.into() })
    }
}

/// An unknown canonical rule name encountered during parsing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown rule {name}")]
pub struct ParseRuleError {
    name: String,
}
impl ParseRuleError {
    /// Return the original unrecognized name without normalization.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A registered rule with topology-scoped input IDs.
/// Constructing this value alone does not validate inputs or register a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSpec {
    /// Rule to execute.
    pub rule: Rule,
    /// Primary input layer.
    pub layer: LayerId,
    /// Secondary input layer, required only for binary rules.
    pub other: Option<LayerId>,
}
