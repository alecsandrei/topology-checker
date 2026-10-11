use anyhow::Result;
use serde::Serialize;
use std::io::Write;
use topology_checker_core::{Report, RuleReport};

/// The established JSON report shape, with core and input warnings as text.
#[derive(Serialize)]
struct JsonReport<'a> {
    tolerance: f64,
    warnings: &'a [String],
    rules: &'a [RuleReport],
}

pub fn json(output: &mut impl Write, value: &impl serde::Serialize) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    writeln!(output)?;
    Ok(())
}

pub fn print(
    report: &Report,
    warnings: &[String],
    as_json: bool,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<()> {
    if as_json {
        json(
            output,
            &JsonReport {
                tolerance: report.tolerance,
                warnings,
                rules: &report.rules,
            },
        )?;
    } else {
        writeln!(output, "Tolerance: {}", report.tolerance)?;
        writeln!(output, "{:<55} {:<30} Errors", "Rule", "Layers")?;
        for row in &report.rules {
            let layers = row
                .other
                .as_ref()
                .map_or_else(|| row.layer.clone(), |b| format!("{}, {b}", row.layer));
            let counts = if row.counts.is_empty() {
                "valid".to_string()
            } else {
                row.counts
                    .iter()
                    .map(|(kind, n)| format!("{}: {n}", kind.name()))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            writeln!(
                output,
                "{:<55} {:<30} {}",
                row.spec.rule.name(),
                layers,
                counts
            )?;
        }
        writeln!(output, "{} topology errors", report.violation_count())?;
    }
    for warning in warnings {
        let _ = writeln!(diagnostics, "warning: {warning}");
    }
    Ok(())
}
