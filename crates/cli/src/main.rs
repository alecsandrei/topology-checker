mod cli;
mod config;
mod read;
mod report;
mod write;
use anyhow::Result;
use clap::ArgMatches;
use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf, process::ExitCode};
use topology_checker_core::{LayerId, Rule, Topology};

fn main() -> ExitCode {
    let command = cli::command();
    let matches = match command.try_get_matches() {
        Ok(m) => m,
        Err(e) => {
            let code = if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                0
            } else {
                2
            };
            let _ = e.print();
            return ExitCode::from(code);
        }
    };
    match run(&matches) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            if matches.get_flag("json") {
                eprintln!("{}", serde_json::json!({"error":format!("{e:#}")}));
            } else {
                eprintln!("error: {e:#}");
            }
            ExitCode::from(2)
        }
    }
}
fn run(matches: &ArgMatches) -> Result<u8> {
    let (command, args) = matches.subcommand().expect("clap requires a command");
    match command {
        "rules" => {
            #[derive(Serialize)]
            struct Info {
                name: &'static str,
                description: &'static str,
                arcgis: Option<&'static str>,
                qgis: Option<&'static str>,
            }
            let rules = Rule::ALL
                .iter()
                .map(|r| Info {
                    name: r.name(),
                    description: r.description(),
                    arcgis: r.arcgis(),
                    qgis: r.qgis(),
                })
                .collect::<Vec<_>>();
            if matches.get_flag("json") {
                println!("{}", serde_json::to_string(&rules)?);
            } else {
                for r in rules {
                    println!("{:<55} {}", r.name, r.description);
                }
            }
            Ok(0)
        }
        "drivers" => {
            let drivers = write::drivers(args.get_flag("read"), args.get_flag("write"));
            if matches.get_flag("json") {
                println!("{}", serde_json::to_string(&drivers)?);
            } else {
                for d in drivers {
                    println!(
                        "{:<22} read={} write={} {}",
                        d.name,
                        d.read,
                        d.write,
                        d.extensions.join(" ")
                    );
                }
            }
            Ok(0)
        }
        "validate" => validate_config(matches, args),
        "utilities" => utility(matches, args),
        kind => {
            let (name, rule_args) = args.subcommand().expect("clap requires a rule");
            let rule = format!("{kind}/{name}")
                .parse::<Rule>()
                .map_err(anyhow::Error::msg)?;
            let input = rule_args
                .get_one::<PathBuf>("input")
                .expect("required input");
            let mut inputs = read::read(
                input,
                rule_args.get_one::<String>("layer").map(String::as_str),
                &input.display().to_string(),
            )?;
            let count = inputs.len();
            if rule.inputs().1.is_some() {
                let other = rule_args
                    .get_one::<PathBuf>("other")
                    .expect("required other");
                inputs.extend(read::read(
                    other,
                    rule_args
                        .get_one::<String>("other-layer")
                        .map(String::as_str),
                    &other.display().to_string(),
                )?);
            }
            let srs = read::common_crs(&inputs)?;
            let tolerance = matches
                .get_one::<f64>("tolerance")
                .copied()
                .unwrap_or_else(|| read::default_tolerance(srs.as_ref()));
            let (mut topology, ids, warnings) = collect(inputs, tolerance)?;
            for &a in &ids[..count] {
                if rule.inputs().1.is_some() {
                    for &b in &ids[count..] {
                        topology.add_rule(rule, a, Some(b))?;
                    }
                } else {
                    topology.add_rule(rule, a, None)?;
                }
            }
            finish(matches, rule_args, &topology, srs.as_ref(), warnings)
        }
    }
}
fn collect(
    inputs: Vec<read::ReadLayer>,
    tolerance: f64,
) -> Result<(Topology, Vec<LayerId>, Vec<String>)> {
    let mut topology = Topology::new(tolerance)?;
    let mut ids = Vec::new();
    let mut warnings = Vec::new();
    for input in inputs {
        warnings.extend(input.warnings);
        ids.push(topology.add_layer(input.layer));
    }
    Ok((topology, ids, warnings))
}
fn validate_config(matches: &ArgMatches, args: &ArgMatches) -> Result<u8> {
    let cfg = config::Config::read(args.get_one::<PathBuf>("input").expect("required config"))?;
    let mut inputs = Vec::new();
    let mut ranges = BTreeMap::new();
    for (alias, input) in cfg.layers {
        let start = inputs.len();
        inputs.extend(read::read(&input.path, input.layer.as_deref(), &alias)?);
        ranges.insert(alias, start..inputs.len());
    }
    let srs = read::common_crs(&inputs)?;
    let tolerance = matches
        .get_one::<f64>("tolerance")
        .copied()
        .or(cfg.tolerance)
        .unwrap_or_else(|| read::default_tolerance(srs.as_ref()));
    let (mut topology, ids, warnings) = collect(inputs, tolerance)?;
    for check in cfg.rules {
        for &a in &ids[ranges[&check.layer].clone()] {
            if let Some(other) = &check.other {
                for &b in &ids[ranges[other].clone()] {
                    topology.add_rule(check.rule, a, Some(b))?;
                }
            } else {
                topology.add_rule(check.rule, a, None)?;
            }
        }
    }
    finish(matches, args, &topology, srs.as_ref(), warnings)
}
fn finish(
    matches: &ArgMatches,
    args: &ArgMatches,
    topology: &Topology,
    srs: Option<&gdal::spatial_ref::SpatialRef>,
    warnings: Vec<String>,
) -> Result<u8> {
    let mut result = topology.validate()?;
    result.warnings.extend(warnings);
    if srs.is_some_and(|s| s.is_geographic()) && result.tolerance > 0. {
        result
            .warnings
            .push("tolerance is in angular units; calculations are planar".into());
    }
    if let Some(path) = args.get_one::<PathBuf>("output") {
        write::report(
            path,
            matches.get_one::<String>("driver").map(String::as_str),
            matches.get_flag("overwrite"),
            srs,
            &result,
            topology,
        )?;
        if !result.is_valid()
            && matches
                .get_one::<String>("driver")
                .is_some_and(|s| s == "ESRI Shapefile")
            || (!result.is_valid() && path.extension().is_some_and(|e| e == "shp"))
        {
            result.warnings.push("Shapefile fields use orig_layer, dest_layer and dest_fid to fit the 10-character limit".into());
        }
    }
    report::print(&result, matches.get_flag("json"))?;
    Ok(if result.is_valid() { 0 } else { 1 })
}

fn utility(matches: &ArgMatches, args: &ArgMatches) -> Result<u8> {
    let (name, args) = args.subcommand().expect("required utility");
    let input = args.get_one::<PathBuf>("input").expect("required input");
    let layers = read::read(
        input,
        args.get_one::<String>("layer").map(String::as_str),
        &input.display().to_string(),
    )?;
    let srs = read::common_crs(&layers)?;
    let tolerance = matches
        .get_one::<f64>("tolerance")
        .copied()
        .unwrap_or_else(|| read::default_tolerance(srs.as_ref()));
    let (mut t, ids, warnings) = collect(layers, tolerance)?;
    let mut geometries = Vec::new();
    fn extract(g: &geo::Geometry, lines: &mut Vec<geo::LineString>) {
        match g {
            geo::Geometry::Line(l) => lines.push((*l).into()),
            geo::Geometry::LineString(l) => lines.push(l.clone()),
            geo::Geometry::MultiLineString(m) => lines.extend(m.0.iter().cloned()),
            geo::Geometry::GeometryCollection(c) => {
                for g in c {
                    extract(g, lines);
                }
            }
            _ => {}
        }
    }
    for id in ids {
        t.add_rule(Rule::LineMustNotHaveDangles, id, None)?;
        let mut lines = Vec::new();
        for (_, g) in t.layer(id)?.features() {
            extract(g, &mut lines);
        }
        let output = if name == "merge-lines" {
            topology_checker_core::merge_lines(lines, tolerance)?
        } else {
            topology_checker_core::explode_lines(lines)?
        };
        geometries.extend(output.into_iter().map(geo::Geometry::LineString));
    }
    let output = args.get_one::<PathBuf>("output").expect("required output");
    let paths = write::geometries(
        output,
        matches.get_one::<String>("driver").map(String::as_str),
        matches.get_flag("overwrite"),
        srs.as_ref(),
        &geometries,
        if name == "merge-lines" {
            "merged"
        } else {
            "exploded"
        },
    )?;
    if matches.get_flag("json") {
        println!(
            "{}",
            serde_json::json!({"outputs":paths,"features":geometries.len(),"tolerance":tolerance,"warnings":warnings})
        );
    } else {
        println!("wrote {} line features", geometries.len());
    }
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
    Ok(0)
}
