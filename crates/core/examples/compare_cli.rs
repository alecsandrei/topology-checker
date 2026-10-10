//! Generate the Criterion workloads as GeoJSON and time both complete CLIs without exports.
//! Usage: cargo run -p topology-checker-core --release --example compare_cli -- OLD NEW DIRECTORY
use std::{
    env,
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    process::{Command, Stdio},
    time::Instant,
};
fn dataset(path: &Path, kind: &str, n: usize) -> std::io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    write!(out, r#"{{"type":"FeatureCollection","features":["#)?;
    for i in 0..n {
        if i > 0 {
            write!(out, ",")?;
        }
        write!(
            out,
            r#"{{"type":"Feature","id":{i},"properties":{{}},"geometry":{{"type":"{kind}","coordinates":"#
        )?;
        match kind {
            "LineString" => write!(out, "[[{i},0],[{},0]]", i + 1)?,
            "Polygon" => {
                let x = i as f64 * 1.5;
                write!(
                    out,
                    "[[[{x},0],[{},0],[{},2],[{x},2],[{x},0]]]",
                    x + 2.,
                    x + 2.
                )?;
            }
            _ => write!(out, "[{i},0]")?,
        }
        write!(out, "}}}}")?;
    }
    write!(out, "]}}")?;
    out.flush()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("expected OLD_BINARY NEW_BINARY DIRECTORY".into());
    }
    let directory = Path::new(&args[2]);
    fs::create_dir_all(directory)?;
    println!("rule,n,binary,seconds,exit");
    for n in [40_000, 80_000, 160_000] {
        let area = directory.join(format!("area-{n}.geojson"));
        fs::write(
            &area,
            format!(
                r#"{{"type":"FeatureCollection","features":[{{"type":"Feature","properties":{{}},"geometry":{{"type":"Polygon","coordinates":[[[-1,-1],[{},-1],[{},1],[-1,1],[-1,-1]]]}}}}]}}"#,
                n + 1,
                n + 1
            ),
        )?;
        for (group, rule, kind) in [
            ("line", "must-not-have-dangles", "LineString"),
            ("polygon", "must-not-overlap", "Polygon"),
            ("point", "must-be-inside", "Point"),
        ] {
            let input = directory.join(format!("{group}-{n}.geojson"));
            dataset(&input, kind, n)?;
            for (version, binary) in args[..2].iter().enumerate() {
                let mut command = Command::new(binary);
                if version == 0 {
                    command.arg("--use-gdal");
                } else {
                    command.args(["--tolerance", "0"]);
                }
                command.args([group, rule]);
                if version == 0 {
                    command.arg(format!("INPUT={}", input.display()));
                    if group == "point" {
                        command.arg(format!("POLYGONS={}", area.display()));
                    }
                } else {
                    command.arg(&input);
                    if group == "point" {
                        command.arg("--other").arg(&area);
                    }
                }
                command.stdout(Stdio::null()).stderr(Stdio::inherit());
                let status = command.status()?;
                if !matches!(status.code(), Some(0 | 1)) {
                    return Err(format!("{binary} failed during warmup: {status}").into());
                }
                for _ in 0..3 {
                    let start = Instant::now();
                    let status = command.status()?;
                    if !matches!(status.code(), Some(0 | 1)) {
                        return Err(format!("{binary} failed: {status}").into());
                    }
                    println!(
                        "{group}/{rule},{n},{}, {:.6},{}",
                        if version == 0 { "old" } else { "new" },
                        start.elapsed().as_secs_f64(),
                        status.code().unwrap()
                    );
                }
            }
        }
    }
    Ok(())
}
