//! Preload via the production importer; accept one sample request per stdin line.
use std::{
    env,
    io::{self, BufRead, Write},
    path::Path,
    time::Instant,
};
use topology_checker_core::{Layer, Rule, Topology};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.as_slice() == ["catalog"] {
        println!(
            "{}",
            serde_json::to_string(&Rule::ALL.iter().map(|r| r.name()).collect::<Vec<_>>())?
        );
        return Ok(());
    }
    anyhow::ensure!(
        args.len() == 2 || args.len() == 3,
        "expected RULE INPUT [OTHER]"
    );
    let rule: Rule = args[0].parse()?;
    let load = |path: &str, name: &str| -> anyhow::Result<_> {
        let mut imported = topology_checker_io::read::read(Path::new(path), None, name)?;
        anyhow::ensure!(imported.len() == 1, "expected one layer");
        let source = imported.remove(0);
        anyhow::ensure!(
            source.warnings.is_empty(),
            "import warnings: {:?}",
            source.warnings
        );
        Ok(source
            .layer
            .features()
            .map(|(fid, g)| (fid, g.clone()))
            .collect::<Vec<_>>())
    };
    let primary = load(&args[1], "input")?;
    let secondary = args.get(2).map(|p| load(p, "other")).transpose()?;
    println!(
        "{}",
        serde_json::json!({"status":"ready", "threads": env::var("RAYON_NUM_THREADS")?,
            "package_version": env!("CARGO_PKG_VERSION"), "gdal": gdal::version_info("RELEASE_NAME")})
    );
    io::stdout().flush()?;
    for request in io::stdin().lock().lines() {
        request?;
        // Fresh caches and geometry copies are prepared outside the measured scope.
        let mut topology = Topology::new(0.0)?;
        let a = topology.add_layer(Layer::from_features("input", primary.clone()));
        let b = secondary
            .as_ref()
            .map(|features| topology.add_layer(Layer::from_features("other", features.clone())));
        println!("{}", serde_json::json!({"status": "validating"}));
        io::stdout().flush()?;
        let start = Instant::now();
        topology.add_rule(rule, a, b)?;
        let report = topology.validate()?;
        let seconds = start.elapsed().as_secs_f64();
        println!("{}", serde_json::json!({"status": "extracting"}));
        io::stdout().flush()?;
        let errors: Vec<_> = report
            .violations()
            .map(|e| {
                serde_json::json!({
                    "kind": e.kind.name(),
                    "id": e.origin.as_ref().map(|f| f.fid),
                    "destination": e.destination.as_ref().map(|f| f.fid),
                    "layer": e.origin.as_ref().map(|f| f.layer.index())
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({"status":"ok", "seconds":seconds, "raw_count":report.violation_count(), "errors":errors})
        );
        io::stdout().flush()?;
    }
    Ok(())
}
