use anyhow::Result;
use std::io::Write;
use topology_checker_io::report_store::DiskReport;

pub fn json(output: &mut impl Write, value: &impl serde::Serialize) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    writeln!(output)?;
    Ok(())
}

pub fn print(
    report: &DiskReport,
    warnings: &[String],
    as_json: bool,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<()> {
    if as_json {
        output.write_all(b"{\"tolerance\":")?;
        serde_json::to_writer(&mut *output, &report.summary.tolerance)?;
        output.write_all(b",\"warnings\":")?;
        serde_json::to_writer(&mut *output, warnings)?;
        output.write_all(b",\"rules\":[")?;
        let mut rows = report.rows();
        for (i, rule) in report.summary.rules.iter().enumerate() {
            if i > 0 {
                output.write_all(b",")?;
            }
            output.write_all(b"{\"spec\":")?;
            serde_json::to_writer(&mut *output, &rule.spec)?;
            output.write_all(b",\"layer\":")?;
            serde_json::to_writer(&mut *output, &rule.layer)?;
            output.write_all(b",\"other\":")?;
            serde_json::to_writer(&mut *output, &rule.other)?;
            output.write_all(b",\"counts\":")?;
            serde_json::to_writer(&mut *output, &rule.counts)?;
            output.write_all(b",\"errors\":[")?;
            for j in 0..rule.total {
                if j > 0 {
                    output.write_all(b",")?;
                }
                let (row_rule, v) = rows.next().expect("stored rule row count")?;
                anyhow::ensure!(row_rule == i, "stored rule order");
                serde_json::to_writer(&mut *output, &v)?;
            }
            output.write_all(b"]}")?;
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
