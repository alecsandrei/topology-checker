use anyhow::Result;
use std::io::Write;
use topology_checker_core::Report;

pub fn json(output: &mut impl Write, value: &impl serde::Serialize) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    writeln!(output)?;
    Ok(())
}

pub fn print(
    report: &Report,
    as_json: bool,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<()> {
    if as_json {
        json(output, report)?;
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
        writeln!(output, "{} topology errors", report.error_count())?;
    }
    for warning in &report.warnings {
        let _ = writeln!(diagnostics, "warning: {warning}");
    }
    Ok(())
}
