use anyhow::Result;
use topology_checker_core::Report;

pub fn print(report: &Report, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(report)?);
    } else {
        println!("Tolerance: {}", report.tolerance);
        println!("{:<55} {:<30} Errors", "Rule", "Layers");
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
            println!("{:<55} {:<30} {}", row.spec.rule.name(), layers, counts);
        }
        println!("{} topology errors", report.error_count());
    }
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}
