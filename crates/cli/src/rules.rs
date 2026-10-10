use std::io::{self, Write};
use topology_checker_core::{GeometryKind, Rule};

pub(super) fn render(output: &mut impl Write) -> io::Result<()> {
    writeln!(output, "Available topology rules ({})\n", Rule::ALL.len())?;
    for (kind, heading) in [
        (GeometryKind::Point, "Points"),
        (GeometryKind::Line, "Lines"),
        (GeometryKind::Polygon, "Polygons"),
        (GeometryKind::Any, "All geometries"),
    ] {
        let rules = || Rule::ALL.iter().copied().filter(|r| r.inputs().0 == kind);
        writeln!(output, "{heading} ({})\n", rules().count())?;
        for rule in rules() {
            writeln!(output, "  {}", rule.name())?;
            explanation(output, description(rule))?;
            let (first, second) = rule.inputs();
            match second {
                None => writeln!(output, "    Needs: one {}", layer(first))?,
                Some(GeometryKind::Any) if first == GeometryKind::Any => {
                    writeln!(output, "    Needs: two layers of any geometry type")?;
                }
                Some(other) if other == first => {
                    writeln!(output, "    Needs: two {}s", layer(first))?;
                }
                Some(other) => {
                    writeln!(output, "    Needs: {} + {}", layer(first), layer(other))?;
                }
            }
            writeln!(output)?;
        }
    }
    writeln!(output, "Usage: topology-checker <group> <rule> <input>")?;
    writeln!(output, "For two layers, add: --other <input>")?;
    writeln!(output, "Rule help: topology-checker <group> <rule> --help")
}

fn layer(kind: GeometryKind) -> &'static str {
    match kind {
        GeometryKind::Point => "point layer",
        GeometryKind::Line => "line layer",
        GeometryKind::Polygon => "polygon layer",
        GeometryKind::Collection => "geometry collection layer",
        GeometryKind::Any => "layer of any geometry type",
    }
}

fn explanation(output: &mut impl Write, text: &str) -> io::Result<()> {
    let mut column = 4;
    write!(output, "    ")?;
    for word in text.split_whitespace() {
        if column > 4 {
            if column + 1 + word.len() > 80 {
                write!(output, "\n    ")?;
                column = 4;
            } else {
                write!(output, " ")?;
                column += 1;
            }
        }
        write!(output, "{word}")?;
        column += word.len();
    }
    writeln!(output)
}

const fn description(rule: Rule) -> &'static str {
    match rule {
        Rule::PointMustNotOverlap => "Find duplicate points within the distance tolerance.",
        Rule::PointMustNotOverlapWith => {
            "Find duplicate points across two layers within the distance tolerance."
        }
        Rule::PointMustBeInside => {
            "Find points outside the polygons, including points on or within the distance tolerance of their edges."
        }
        Rule::LineMustNotHaveDangles => "Find unconnected line endpoints.",
        Rule::LineMustNotHavePseudoNodes => {
            "Find joins connecting exactly two distinct features at their endpoints. Closed loops are allowed."
        }
        Rule::LineMustNotOverlap => {
            "Find shared line segments between distinct features. Crossings and overlaps within one feature are excluded."
        }
        Rule::LineMustNotOverlapWith => {
            "Find shared line segments across two layers. Crossings are excluded."
        }
        Rule::LineMustNotSelfOverlap => {
            "Find line segments traced more than once by the same feature, including across its parts."
        }
        Rule::LineMustNotIntersect => {
            "Find overlaps and crossings between distinct features away from their endpoints. Endpoint touches are allowed."
        }
        Rule::LineMustNotIntersectWith => {
            "Find overlaps and crossings across two layers away from their endpoints. Endpoint touches are allowed."
        }
        Rule::LineMustNotIntersectOrTouchInterior => {
            "Find overlaps, crossings and endpoint touches on the interior of another feature. Touches only at both features' endpoints are allowed."
        }
        Rule::LineMustNotIntersectOrTouchInteriorWith => {
            "Find overlaps, crossings and endpoint touches on a line's interior across two layers. Touches only at both features' endpoints are allowed."
        }
        Rule::LineMustNotSelfIntersect => {
            "Find overlaps and crossings within one feature. Closed loops and self-touches are allowed."
        }
        Rule::LineMustBeInside => {
            "Find line pieces outside the polygons that are longer than the distance tolerance. Lines on polygon edges are allowed."
        }
        Rule::PolygonMustNotOverlap => {
            "Find shared areas between distinct polygon features, ignoring slivers below the tolerance."
        }
        Rule::PolygonMustNotOverlapWith => {
            "Find shared polygon areas across two layers, ignoring slivers below the tolerance."
        }
        Rule::PolygonMustNotHaveGaps => {
            "Find enclosed gaps between polygons, ignoring sliver gaps below the tolerance. Also report outer boundaries separately."
        }
        Rule::GeometryMustNotBeMultipart => {
            "Find features with more than one part. A Multi geometry with only one part is allowed."
        }
        Rule::PointMustBeCoveredByLines => {
            "Find points without covering linework, including line endpoints."
        }
        Rule::PointMustBeCoveredByLineEndpoints => {
            "Find points without a covering open line endpoint. Closed line parts have no endpoints."
        }
        Rule::PointMustBeCoveredByPolygonBoundaries => {
            "Find points away from polygon edges, including holes and shared parcel edges."
        }
        Rule::LineEndpointsMustBeCoveredByPoints => {
            "Find open line endpoints without a covering point. Closed line parts have no endpoints."
        }
        Rule::LineMustBeCoveredByPolygonBoundaries => {
            "Find line pieces that do not follow polygon edges, including holes and shared edges."
        }
        Rule::PolygonBoundariesMustBeCoveredByLines => {
            "Find polygon edge pieces without covering linework, including hole edges."
        }
        Rule::PolygonMustContainPoint => {
            "Find polygons without a strictly interior point feature. All parts of a point feature must qualify."
        }
        Rule::PolygonMustContainOnePoint => {
            "Find polygons and point features without a one-to-one interior assignment. Count feature IDs, including multipart features."
        }
        Rule::LineMustNotCrossPolygonBoundaries => {
            "Find lines entering or leaving polygons through exterior or hole edges. Tangencies, endpoint touches and shared segments are allowed."
        }
        Rule::GeometryMustNotBeContainedBy => {
            "Find whole features contained by an individual feature in the other layer. Equal geometries are allowed. No tolerance buffer is used."
        }
        Rule::PointMustBeCoveredByPolygons => {
            "Find points outside polygon areas and farther than the tolerance from their edges. Boundary points are allowed."
        }
        Rule::PointMustNotBeInsidePolygons => {
            "Find points strictly inside polygons. Points on or within tolerance of their edges are allowed."
        }
        Rule::PointMustNotIntersectLines => {
            "Find points touching linework anywhere, including endpoints, within tolerance."
        }
        Rule::PointMustNotIntersectPolygons => {
            "Find points inside polygons or touching their edges within tolerance."
        }
        Rule::LineMustBeProperlyInsidePolygons => {
            "Find whole line parts that leave polygon coverage or come within tolerance of its boundary."
        }
        Rule::LineMustNotOverlapWithPolygons => {
            "Find line pieces inside polygons or along their edges, longer than tolerance. Isolated contacts are allowed."
        }
        Rule::LineMustNotOverlapPolygonBoundaries => {
            "Find line pieces shared with polygon edges, longer than tolerance. Crossings and isolated contacts are allowed."
        }
        Rule::LineMustNotIntersectPolygons => {
            "Find any line contact with polygons, including interior pieces, shared edges and isolated touches within tolerance."
        }
        Rule::LineEndpointsMustBeCoveredByPolygonBoundaries => {
            "Find open line endpoints away from polygon exterior or hole edges. Closed line parts have no endpoints."
        }
        Rule::PolygonMustNotContainPoints => {
            "Find strictly contained points, with the polygon as the primary feature. Boundary points are allowed."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explanations_wrap_at_80_columns_without_losing_words() {
        for &rule in Rule::ALL {
            let mut output = Vec::new();
            explanation(&mut output, description(rule)).unwrap();
            let text = String::from_utf8(output).unwrap();
            assert!(text.ends_with('\n'));
            assert!(
                text.lines()
                    .all(|line| line.starts_with("    ") && line.len() <= 80)
            );
            assert_eq!(
                text.split_whitespace().collect::<Vec<_>>(),
                description(rule).split_whitespace().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn wrapping_includes_indentation_and_keeps_exact_width_lines() {
        let word = "x".repeat(74);
        let mut output = Vec::new();
        explanation(&mut output, &format!("{word} y z")).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("    {word} y\n    z\n")
        );
    }

    #[test]
    fn rule_help_keeps_technical_registry_descriptions() {
        let command = crate::cli::command();
        for &rule in Rule::ALL {
            let (group, name) = rule.name().split_once('/').unwrap();
            let help = command
                .find_subcommand(group)
                .unwrap()
                .find_subcommand(name)
                .unwrap();
            assert_eq!(help.get_about().unwrap().to_string(), rule.description());
        }
    }
}
