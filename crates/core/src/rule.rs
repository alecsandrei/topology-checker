use crate::{GeometryKind, LayerId};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

macro_rules! registry {
    ($(($variant:ident, $name:literal, $a:ident, $b:expr, $description:literal, $arcgis:expr, $qgis:expr)),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub enum Rule { $( #[doc = $description] #[serde(rename = $name)] $variant, )* }
        impl Rule {
            pub const ALL: &'static [Self] = &[$(Self::$variant),*];
            pub const fn name(self) -> &'static str { match self { $(Self::$variant => $name),* } }
            pub const fn inputs(self) -> (GeometryKind, Option<GeometryKind>) { match self { $(Self::$variant => (GeometryKind::$a, $b)),* } }
            pub const fn description(self) -> &'static str { match self { $(Self::$variant => $description),* } }
            pub const fn arcgis(self) -> Option<&'static str> { match self { $(Self::$variant => $arcgis),* } }
            pub const fn qgis(self) -> Option<&'static str> { match self { $(Self::$variant => $qgis),* } }
        }
    }
}
use GeometryKind::{Line, Point, Polygon};
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
}
impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
impl FromStr for Rule {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|r| r.name() == value)
            .ok_or_else(|| format!("unknown rule {value}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSpec {
    pub rule: Rule,
    pub layer: LayerId,
    pub other: Option<LayerId>,
}
