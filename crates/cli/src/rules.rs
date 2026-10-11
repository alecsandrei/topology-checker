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
            explanation(output, rule.summary())?;
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
        _ => "layer",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explanations_wrap_at_80_columns_without_losing_words() {
        for &rule in Rule::ALL {
            let mut output = Vec::new();
            explanation(&mut output, rule.summary()).unwrap();
            let text = String::from_utf8(output).unwrap();
            assert!(text.ends_with('\n'));
            assert!(
                text.lines()
                    .all(|line| line.starts_with("    ") && line.len() <= 80)
            );
            assert_eq!(
                text.split_whitespace().collect::<Vec<_>>(),
                rule.summary().split_whitespace().collect::<Vec<_>>()
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
