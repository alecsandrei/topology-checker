# Cross-geometry topology checks

ArcGIS and QGIS both support checks between different geometry types. Support is
organized by the relationship being checked: containment, boundary coverage,
endpoint coverage, or intersection. A check between two layers does not necessarily
accept two different geometry types.

This comparison was checked on 2026-10-10 against ArcGIS Pro's current documentation
and the QGIS 4.2 manual. QGIS implementation details below are explicitly tied to
the 3.44 source where cited. These are documentation comparisons, not claims that
the products have identical tolerance or error-reporting behavior.

## Named topology rules

ArcGIS geodatabase topology requires a Standard or Advanced license. Its rules can
relate different feature classes and subtypes, with validation errors stored in the
geodatabase. See the [ArcGIS topology rule documentation](https://doc.esri.com/en/arcgis-pro/latest/help/editing/geodatabase-topology-rules-for-polyline-features.html).

The QGIS column below refers specifically to its **Topology Checker** core plugin.
"Not listed" means absent from that plugin's documented rule list, not unavailable
through other QGIS tools. The manual describes the point coverage rule's target as
a vector layer without specifying every permitted geometry family. See the
[QGIS rule catalog](https://docs.qgis.org/4.2/en/docs/user_manual/plugins/core_plugins/plugins_topology_checker.html).

The first input in each row is the feature being constrained. Reversing the inputs
changes the check: requiring every point to lie inside a polygon does not require
every polygon to contain a point.

| Required relationship | Inputs | ArcGIS Pro rule | QGIS Topology Checker | This checker |
| --- | --- | --- | --- | --- |
| Each point lies somewhere on a line | Point, line | [Must Be Covered By](https://doc.esri.com/en/arcgis-pro/latest/tool-reference/data-management/add-rule-to-topology.html) | Must be covered by (see target-type note above) | `point/must-be-covered-by-lines` |
| Each point coincides with a line endpoint | Point, line | [Must Be Covered By Endpoint Of](https://doc.esri.com/en/arcgis-pro/latest/help/editing/geodatabase-topology-rules-for-point-features.html) | Must be covered by endpoints of | `point/must-be-covered-by-line-endpoints` |
| Each line endpoint has a point | Line, point | [Endpoint Must Be Covered By](https://doc.esri.com/en/arcgis-pro/latest/help/editing/geodatabase-topology-rules-for-polyline-features.html) | End points must be covered by | `line/endpoints-must-be-covered-by-points` |
| Each point is strictly inside a polygon, excluding its boundary | Point, polygon | [Must Be Properly Inside Polygons](https://doc.esri.com/en/arcgis-pro/latest/help/editing/geodatabase-topology-rules-for-point-features.html) | Must be inside | `point/must-be-inside` |
| Each point lies on a polygon boundary | Point, polygon | [Must Be Covered By Boundary Of](https://doc.esri.com/en/arcgis-pro/latest/help/editing/geodatabase-topology-rules-for-point-features.html) | Not listed | `point/must-be-covered-by-polygon-boundaries` |
| Each polygon contains at least one interior point | Polygon, point | [Contains Point](https://doc.esri.com/en/arcgis-pro/latest/help/editing/geodatabase-topology-rules-for-polygon-features.html) | Must contain | `polygon/must-contain-point` |
| Each polygon contains exactly one point; each point belongs to exactly one polygon | Polygon, point | [Contains One Point](https://doc.esri.com/en/arcgis-pro/latest/tool-reference/data-management/add-rule-to-topology.html) | Not listed | `polygon/must-contain-one-point` |
| Lines stay within polygons; polygon edges are allowed | Line, polygon | [Must Be Inside](https://doc.esri.com/en/arcgis-pro/latest/tool-reference/data-management/add-rule-to-topology.html) | Not listed | `line/must-be-inside` |
| Every line segment follows a polygon boundary | Line, polygon | [Must Be Covered By Boundary Of](https://doc.esri.com/en/arcgis-pro/latest/help/editing/geodatabase-topology-rules-for-polyline-features.html) | Not listed | `line/must-be-covered-by-polygon-boundaries` |
| Every polygon boundary segment has a covering line | Polygon, line | [Boundary Must Be Covered By](https://doc.esri.com/en/arcgis-pro/latest/help/editing/geodatabase-topology-rules-for-polygon-features.html) | Not listed | `polygon/boundaries-must-be-covered-by-lines` |

The QGIS plugin manual gives less detail about boundary behavior for point
containment than ArcGIS. Its separate Geometry Checker explicitly offers strict
point containment; see the [QGIS geometry validation options](https://docs.qgis.org/4.2/en/docs/user_manual/plugins/core_plugins/plugins_geometry_checker.html).

## Other QGIS checks

QGIS's **Geometry Checker** also offers point-on-line validation, strict point
containment, line intersections against another layer, and polygon boundary
following. These are separate from the Topology Checker rule menu. See the
[Geometry Checker manual](https://docs.qgis.org/4.2/en/docs/user_manual/plugins/core_plugins/plugins_geometry_checker.html).

The **Processing Toolbox** exposes these relevant validation algorithms, documented
as added in QGIS 3.42:

| Algorithm | Inputs | What it reports |
| --- | --- | --- |
| `native:checkgeometrypointcoveredbyline` | Points and line layers | Points without a covering line |
| `native:checkgeometrypointinpolygon` | Points and polygon layers | Points that fail polygon containment |
| `native:checkgeometrylinelayerintersection` | Lines and a line or polygon check layer | Intersection locations as points, with optional affected lines |
| `native:checkgeometrycontained` | Any geometry and polygon layers | Contained features, treating containment as an error |

The last algorithm detects unwanted contained features. It does not check that
every input line stays inside polygons, and it is not a general partial-overlap
check. Input types, outputs and algorithm IDs come from the
[QGIS Processing validation reference](https://docs.qgis.org/4.2/en/docs/user_manual/processing_algs/qgis/checkgeometry.html).

For line-versus-polygon intersections, the QGIS 3.44 implementation checks the line
against polygon rings and returns intersection points. A line entirely inside a
polygon without meeting its boundary therefore is not detected by this particular
check. This conclusion follows from the
[QGIS implementation](https://api.qgis.org/api/3.44/qgsgeometrylinelayerintersectioncheck_8cpp_source.html),
and should not be generalized to every QGIS intersection tool or version.

QGIS also offers **Select by location** and **Extract by location** across vector
layers, with predicates including intersection, containment, touching, crossing,
and being within. These support custom cross-geometry workflows but require the
user to decide which relationships constitute errors. See the
[QGIS spatial selection reference](https://docs.qgis.org/4.2/en/docs/user_manual/processing_algs/qgis/vectorselection.html).

## What line-versus-polygon "overlap" means

ArcGIS's named prohibition on overlap between layers accepts line/line or
polygon/polygon inputs, rather than a generic line/polygon pair. Its mixed-type
rules above use explicit containment and boundary relationships. See the
[ArcGIS rule input types](https://doc.esri.com/en/arcgis-pro/latest/tool-reference/data-management/add-rule-to-topology.html).

The catalog now exposes three distinct prohibitions:

| Rule | Violation | Allowed |
| --- | --- | --- |
| `line/must-not-overlap-polygon-boundaries` | Shared individual boundary segments longer than tolerance | Crossings and isolated touches |
| `line/must-not-overlap-with-polygons` | Positive-length pieces inside areas or along boundaries, longer than tolerance | Isolated contacts |
| `line/must-not-intersect-polygons` | Interior pieces, shared segments, crossings and isolated contacts | Disjoint linework |

`line/must-not-cross-polygon-boundaries` specifically reports entry and exit through
exterior and hole rings, including at vertices. Tangencies, endpoint touches and
shared paths are allowed. It is therefore narrower than QGIS's documented
intersection validation and has no claimed exact vendor equivalent in the catalog.
A line entirely inside a polygon fails the area overlap and any-contact checks,
while it passes the boundary crossing and boundary overlap checks.

## Implemented semantics

All 20 additions are binary rules. CLI calls use `--other`; TOML rules use
`layer` and `other`; library callers use `Topology::add_rule` with a secondary ID.
The existing 18 entries remain an unchanged registry prefix, bringing the total to
38. Catalog JSON and report JSON keep their existing fields. The public `Rule`
and `ErrorType` enums gained variants, so exhaustive Rust matches need updating.
New error names are `not-covered`, `missing-point`, `point-count`, `contained` and
`intersection`.

Point-to-line, endpoint and boundary coverage accept distances at or below the
planar tolerance. Strict interior points must be farther than tolerance from all
boundaries. Polygon area coverage accepts interior points and boundary proximity,
including hole boundaries. Prohibitions distinguish strictly interior points from
any contact. Boundary coverage uses individual polygon rings, retaining shared
parcel edges. Required line containment uses the polygon layer union;
`line/must-be-properly-inside-polygons` additionally excludes every boundary contact
and reports each failing whole part.

New line uncovered and overlap pieces must be longer than tolerance. Any-contact
validation retains isolated contacts even if an associated piece is too short to
report as a line. Mixed contact geometries remain geometry collections containing
both points and lines. Pair violations use individual polygon features, preserving
origin and destination feature IDs and original part indices.

Polygon cardinality counts distinct point feature IDs. Every retained point part
must qualify within the polygon feature's combined parts. Disconnected polygon
parts with one FID count as one polygon. The one-point rule also reports unassigned
and multiply assigned point features: these errors have `destination` provenance
and no `origin`. Polygon failures have `origin` provenance. Closed line parts have
no endpoints. Empty and degenerate geometries follow existing cleaning and warning
behavior. Empty target layers fail requirements and pass prohibitions.

`geometry/must-not-be-contained-by` checks whole cleaned features against each
individual target feature, excluding mutual containment (geometric equality).
It uses topological `contains` with no tolerance buffer. It accepts any target
geometry family, broader than QGIS Processing's documented polygon check layer,
and returns the whole feature rather than just a centroid error marker. No rule
claims to reproduce vendor snapping, repairs, exception handling or editing
workflows. QGIS Processing's integer precision tolerance also differs from this
checker's planar distance tolerance.

## Practical additions without vendor equivalents

| Rule | Inputs | Meaning |
| --- | --- | --- |
| `point/must-be-covered-by-polygons` | Point, polygon | Interior or boundary proximity is required |
| `point/must-not-be-inside-polygons` | Point, polygon | Strict interior is forbidden |
| `point/must-not-intersect-lines` | Point, line | Any line contact is forbidden, including endpoints |
| `point/must-not-intersect-polygons` | Point, polygon | Interior or boundary contact is forbidden |
| `line/must-be-properly-inside-polygons` | Line, polygon | Whole parts must remain strictly inside polygon coverage |
| `line/must-not-overlap-with-polygons` | Line, polygon | Interior and shared boundary pieces are forbidden |
| `line/must-not-overlap-polygon-boundaries` | Line, polygon | Only shared boundary pieces are forbidden |
| `line/must-not-intersect-polygons` | Line, polygon | Any contact is forbidden |
| `line/endpoints-must-be-covered-by-polygon-boundaries` | Line, polygon | Open endpoints must lie on polygon rings |
| `polygon/must-not-contain-points` | Polygon, point | Strictly interior point parts are forbidden, polygon is origin |

## Vendor relationship traceability

This matrix covers the mixed-geometry relationships in the references above.
Acceptance tests are in [cross_geometry.rs](../crates/core/tests/cross_geometry.rs)
unless marked as existing tests in [acceptance.rs](../crates/core/tests/acceptance.rs).
`every_new_rule_has_valid_and_invalid_examples` exercises every addition on both
sides of its predicate. Specialized acceptance tests below cover harder cases.

| Documented relationship | Canonical rule | Acceptance test |
| --- | --- | --- |
| ArcGIS point on line; QGIS point covered by; Processing point covered by line | `point/must-be-covered-by-lines` | `point_and_endpoint_coverage_use_inclusive_distance_thresholds` |
| ArcGIS and QGIS point on endpoint | `point/must-be-covered-by-line-endpoints` | `closed_parts_have_no_endpoints_and_cleaning_keeps_original_part_indices` |
| ArcGIS and QGIS endpoints covered by points | `line/endpoints-must-be-covered-by-points` | `closed_parts_have_no_endpoints_and_cleaning_keeps_original_part_indices` |
| ArcGIS strict point containment; QGIS inside; Processing point in polygon | `point/must-be-inside` | Existing `point_boundary_is_outside`; oracle `point_inside_matches_brute_force` |
| ArcGIS point on polygon boundary | `point/must-be-covered-by-polygon-boundaries` | `boundaries_keep_holes_and_shared_edges` |
| ArcGIS contains point; QGIS must contain | `polygon/must-contain-point` | `cardinality_counts_features_and_preserves_input_roles` |
| ArcGIS contains one point | `polygon/must-contain-one-point` | `cardinality_counts_features_and_preserves_input_roles` |
| ArcGIS line inside area | `line/must-be-inside` | Existing `line_inside_uses_union_and_accepts_boundaries` |
| ArcGIS line on area boundary | `line/must-be-covered-by-polygon-boundaries` | `boundaries_keep_holes_and_shared_edges` |
| ArcGIS area boundary covered by lines; Geometry Checker polygon boundary following (linework target) | `polygon/boundaries-must-be-covered-by-lines` | `boundaries_keep_holes_and_shared_edges` |
| QGIS Processing line against polygon rings | `line/must-not-cross-polygon-boundaries` (narrower crossing semantics) | `crossings_distinguish_vertices_tangencies_endpoint_touches_and_shared_paths` |
| QGIS Processing contained features | `geometry/must-not-be-contained-by` (broader target kinds) | `whole_feature_containment_excludes_equality_and_never_uses_target_union_or_buffer` |

Independent property tests in [oracle.rs](../crates/core/tests/oracle.rs) compare
indexed results against brute-force distance, contains and DE-9IM predicates.
[CLI tests](../crates/cli/tests/cli.rs) execute all additions directly and through
TOML, check catalog completeness and export mixed contacts plus cardinality
failures across all three GDAL geometry families. Existing output-error and
broken-pipe tests remain in place.
