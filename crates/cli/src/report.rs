use anyhow::{Result, ensure};
use std::io::Write;
use topology_checker_core::{RuleSummary, Violation};
use topology_checker_io::report_store::{DiskReport, RuleRows};

pub fn json(output: &mut impl Write, value: &impl serde::Serialize) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    writeln!(output)?;
    Ok(())
}

/// Write `{"tolerance":…,"warnings":[…]`, leaving the object open.
fn head(output: &mut impl Write, tolerance: f64, warnings: &[String]) -> Result<()> {
    output.write_all(b"{\"tolerance\":")?;
    serde_json::to_writer(&mut *output, &tolerance)?;
    output.write_all(b",\"warnings\":")?;
    serde_json::to_writer(&mut *output, warnings)?;
    Ok(())
}

/// Write one rule as it appears in the `rules` array of the JSON report.
/// Without `errors`, as with `--summary`, the `errors` key is left out.
fn rule(
    output: &mut impl Write,
    summary: &RuleSummary,
    errors: Option<impl Iterator<Item = Result<Violation>>>,
) -> Result<()> {
    output.write_all(b"{\"spec\":")?;
    serde_json::to_writer(&mut *output, &summary.spec)?;
    output.write_all(b",\"layer\":")?;
    serde_json::to_writer(&mut *output, &summary.layer)?;
    output.write_all(b",\"other\":")?;
    serde_json::to_writer(&mut *output, &summary.other)?;
    output.write_all(b",\"counts\":")?;
    serde_json::to_writer(&mut *output, &summary.counts)?;
    if let Some(errors) = errors {
        output.write_all(b",\"errors\":[")?;
        let mut written = 0;
        for v in errors {
            if written > 0 {
                output.write_all(b",")?;
            }
            serde_json::to_writer(&mut *output, &v?)?;
            written += 1;
        }
        ensure!(written == summary.total, "stored rule row count");
        output.write_all(b"]")?;
    }
    output.write_all(b"}")?;
    Ok(())
}

/// Write one finished rule as its own line of a streamed report.
pub fn stream_rule(
    output: &mut impl Write,
    summary: &RuleSummary,
    errors: Option<RuleRows<'_>>,
) -> Result<()> {
    rule(output, summary, errors)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

/// Write the last line of a streamed report, `{"tolerance":…,"warnings":[…]}`,
/// and the warnings.
pub fn stream_end(
    report: &DiskReport,
    warnings: &[String],
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<()> {
    head(output, report.summary.tolerance, warnings)?;
    output.write_all(b"}\n")?;
    warn(warnings, diagnostics);
    Ok(())
}

fn warn(warnings: &[String], diagnostics: &mut impl Write) {
    for warning in warnings {
        let _ = writeln!(diagnostics, "warning: {warning}");
    }
}

pub fn print(
    report: &DiskReport,
    warnings: &[String],
    as_json: bool,
    summary: bool,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<()> {
    if as_json {
        head(output, report.summary.tolerance, warnings)?;
        output.write_all(b",\"rules\":[")?;
        let mut rows = report.rows();
        for (i, summary_row) in report.summary.rules.iter().enumerate() {
            if i > 0 {
                output.write_all(b",")?;
            }
            let errors = (!summary).then(|| {
                (&mut rows).take(summary_row.total).map(move |row| {
                    let (row_rule, v) = row?;
                    ensure!(row_rule == i, "stored rule order");
                    Ok(v)
                })
            });
            rule(output, summary_row, errors)?;
        }
        output.write_all(b"]}\n")?;
    } else {
        writeln!(output, "Tolerance: {}", report.summary.tolerance)?;
        writeln!(output, "{:<55} {:<30} Errors", "Rule", "Layers")?;
        for row in &report.summary.rules {
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
            let line = format!("{:<55} {:<30} {}", row.spec.rule.name(), layers, counts);
            writeln!(output, "{line}")?;
        }
        writeln!(output, "{} topology errors", report.violation_count())?;
    }
    warn(warnings, diagnostics);
    Ok(())
}
