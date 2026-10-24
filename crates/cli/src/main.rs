mod cli;
mod config;
mod report;
mod rules;
use anyhow::Result;
use clap::ArgMatches;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};
use topology_checker_core::{GeometryKind, LayerId, Rule, RuleSummary, Topology};
use topology_checker_io::{read, write};

fn main() -> ExitCode {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let stderr = io::stderr();
    let mut diagnostics = stderr.lock();
    let command = cli::command();
    let matches = match command.try_get_matches().and_then(cli::check) {
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
            // Use Clap's existing color adapter with our already locked streams.
            let rendered = e.render();
            let result = if e.use_stderr() {
                let mut colored = anstream::AutoStream::auto(diagnostics);
                let _ = write!(colored, "{}", rendered.ansi());
                let _ = colored.flush();
                diagnostics = colored.into_inner();
                Ok(code)
            } else {
                let mut colored = anstream::AutoStream::auto(output);
                write!(colored, "{}", rendered.ansi())
                    .and_then(|()| colored.flush())
                    .map(|()| code)
                    .map_err(Into::into)
            };
            return ExitCode::from(exit_status(result, false, &mut diagnostics));
        }
    };
    ExitCode::from(exit_status(
        run(&matches, &mut output, &mut diagnostics),
        matches.get_flag("json"),
        &mut diagnostics,
    ))
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
            || cause
                .downcast_ref::<serde_json::Error>()
                .is_some_and(|e| e.io_error_kind() == Some(io::ErrorKind::BrokenPipe))
    })
}

fn exit_status(result: Result<u8>, json: bool, diagnostics: &mut impl Write) -> u8 {
    let code = match result {
        Ok(code) => code,
        Err(e) if is_broken_pipe(&e) => 0,
        Err(e) => {
            if json {
                let _ = writeln!(
                    diagnostics,
                    "{}",
                    serde_json::json!({"error":format!("{e:#}")})
                );
            } else {
                let _ = writeln!(diagnostics, "error: {e:#}");
            }
            2
        }
    };
    let _ = diagnostics.flush();
    code
}

fn run(matches: &ArgMatches, output: &mut impl Write, diagnostics: &mut impl Write) -> Result<u8> {
    let result = run_command(matches, output, diagnostics);
    let flushed = output.flush();
    result.and_then(|code| {
        flushed?;
        Ok(code)
    })
}

fn run_command(
    matches: &ArgMatches,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<u8> {
    let (command, args) = matches.subcommand().expect("clap requires a command");
    match command {
        "rules" => {
            #[derive(Serialize)]
            struct Info {
                name: &'static str,
                description: &'static str,
                arcgis: Option<&'static str>,
                qgis: Option<&'static str>,
                layer: GeometryKind,
                other: Option<GeometryKind>,
            }
            let rules = Rule::ALL
                .iter()
                .map(|r| Info {
                    name: r.name(),
                    description: r.description(),
                    arcgis: r.arcgis(),
                    qgis: r.qgis(),
                    layer: r.inputs().0,
                    other: r.inputs().1,
                })
                .collect::<Vec<_>>();
            if matches.get_flag("json") {
                report::json(output, &rules)?;
            } else {
                rules::render(output)?;
            }
            Ok(0)
        }
        "drivers" => {
            let drivers = write::drivers(args.get_flag("read"), args.get_flag("write"));
            if matches.get_flag("json") {
                report::json(output, &drivers)?;
            } else {
                for d in drivers {
                    writeln!(
                        output,
                        "{:<22} read={} write={} {}",
                        d.name,
                        d.read,
                        d.write,
                        d.extensions.join(" ")
                    )?;
                }
            }
            Ok(0)
        }
        "validate" => validate_config(matches, args, output, diagnostics),
        "utilities" => utility(matches, args, output, diagnostics),
        kind => {
            let (name, rule_args) = args.subcommand().expect("clap requires a rule");
            let rule = format!("{kind}/{name}").parse::<Rule>()?;
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
            finish(
                matches,
                rule_args,
                &topology,
                srs.as_ref(),
                warnings,
                output,
                diagnostics,
            )
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
fn validate_config(
    matches: &ArgMatches,
    args: &ArgMatches,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<u8> {
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
    finish(
        matches,
        args,
        &topology,
        srs.as_ref(),
        warnings,
        output,
        diagnostics,
    )
}
fn finish(
    matches: &ArgMatches,
    args: &ArgMatches,
    topology: &Topology,
    srs: Option<&topology_checker_io::Crs>,
    input_warnings: Vec<String>,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<u8> {
    let progress = matches!(args.try_get_one::<bool>("progress"), Ok(Some(true)));
    // `cli::check` only lets --stream through together with --json.
    let stream = matches!(args.try_get_one::<bool>("stream"), Ok(Some(true)));
    let summary = matches.get_flag("summary");
    let rules = topology.rules().len();
    // A reader that goes away stops the stream, not the run: `-o` and the
    // warnings are still written, as when the plain report meets a closed pipe.
    let mut closed = None;
    let result = topology_checker_io::report_store::DiskReport::validate_with_progress(
        topology,
        None,
        |rule, rows| {
            if progress {
                // Progress is advisory, like warnings: an unwritable stderr never fails validation.
                let _ = writeln!(diagnostics, "{}", progress_line(rule, rules));
                let _ = diagnostics.flush();
            }
            if stream && closed.is_none() {
                match report::stream_rule(output, rule, (!summary).then_some(rows)) {
                    Err(e) if is_broken_pipe(&e) => closed = Some(e),
                    written => written?,
                }
            }
            Ok(())
        },
    )?;
    let mut warnings = result
        .summary
        .warnings
        .iter()
        .map(ToString::to_string)
        .chain(input_warnings)
        .collect::<Vec<_>>();
    if srs.is_some_and(|s| s.is_geographic()) && result.summary.tolerance > 0. {
        warnings.push("tolerance is in angular units; calculations are planar".into());
    }
    if let Some(path) = args.get_one::<PathBuf>("output") {
        write::stored_report(
            path,
            matches.get_one::<String>("driver").map(String::as_str),
            matches.get_flag("overwrite"),
            srs,
            &result,
            write::ReportLayout::ByFamily,
        )?;
        if !result.is_valid()
            && matches.get_one::<String>("driver").map_or_else(
                || {
                    path.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("shp"))
                },
                |s| s == "ESRI Shapefile",
            )
        {
            warnings.push("Shapefile fields use viol_id, orig_layer, orig_part, dest_layer, dest_fid and dest_part to fit the 10-character limit".into());
        }
    }
    if stream {
        let end = match closed {
            Some(e) => Err(e),
            None => report::stream_end(&result, &warnings, output),
        };
        report::warn(&warnings, diagnostics);
        end?;
    } else {
        report::print(
            &result,
            &warnings,
            matches.get_flag("json"),
            summary,
            output,
            diagnostics,
        )?;
    }
    Ok(if result.is_valid() { 0 } else { 1 })
}

fn progress_line(rule: &RuleSummary, rules: usize) -> String {
    format!(
        "checked rule {}/{rules} ({}): {} {}",
        rule.spec.id.index() + 1,
        rule.spec.rule.name(),
        rule.total,
        if rule.total == 1 { "error" } else { "errors" }
    )
}

fn utility(
    matches: &ArgMatches,
    args: &ArgMatches,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<u8> {
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
    let (mut t, ids, mut warnings) = collect(layers, tolerance)?;
    let mut geometries = Vec::new();
    for id in ids {
        t.add_rule(Rule::LineMustNotHaveDangles, id, None)?;
        let mut lines = Vec::new();
        for (_, g) in t.layer(id)?.features() {
            extract_lines(g, &mut lines);
        }
        let output = if name == "merge-lines" {
            topology_checker_core::merge_lines(lines, tolerance)?
        } else {
            topology_checker_core::explode_lines(lines)?
        };
        geometries.extend(output.into_iter().map(geo::Geometry::LineString));
    }
    warnings.extend(t.warnings()?.iter().map(ToString::to_string));
    if srs.as_ref().is_some_and(|s| s.is_geographic()) && tolerance > 0. {
        warnings.push("tolerance is in angular units; calculations are planar".into());
    }
    let output_path = args.get_one::<PathBuf>("output").expect("required output");
    let paths = write::geometries(
        output_path,
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
        report::json(
            output,
            &serde_json::json!({"outputs":paths,"features":geometries.len(),"tolerance":tolerance,"warnings":warnings}),
        )?;
    } else {
        writeln!(output, "wrote {} line features", geometries.len())?;
    }
    for warning in warnings {
        let _ = writeln!(diagnostics, "warning: {warning}");
    }
    Ok(0)
}

fn extract_lines(g: &geo::Geometry, lines: &mut Vec<geo::LineString>) {
    match g {
        geo::Geometry::Line(l) => lines.push((*l).into()),
        geo::Geometry::LineString(l) => lines.push(l.clone()),
        geo::Geometry::MultiLineString(m) => lines.extend(m.0.iter().cloned()),
        geo::Geometry::GeometryCollection(c) => {
            for g in c {
                extract_lines(g, lines);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingWriter {
        kind: io::ErrorKind,
        write_limit: Option<usize>,
        flush_fails: bool,
        flushed: bool,
    }
    impl Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(left) = &mut self.write_limit {
                if *left == 0 {
                    return Err(io::Error::from(self.kind));
                }
                let n = bytes.len().min(*left);
                *left -= n;
                Ok(n)
            } else {
                Ok(bytes.len())
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushed = true;
            if self.flush_fails {
                Err(io::Error::from(self.kind))
            } else {
                Ok(())
            }
        }
    }

    fn matches(json: bool, validation: bool) -> ArgMatches {
        let mut args = vec![
            "topology-checker".to_string(),
            "--tolerance".into(),
            "0".into(),
        ];
        if json {
            args.push("--json".into());
        }
        if validation {
            args.extend([
                "point".into(),
                "must-not-overlap".into(),
                format!(
                    "{}/tests/fixtures/points.geojson",
                    env!("CARGO_MANIFEST_DIR")
                ),
            ]);
        } else {
            args.push("rules".into());
        }
        cli::command().try_get_matches_from(args).unwrap()
    }

    #[test]
    fn line_extraction_flattens_every_linear_geometry_and_skips_the_rest() {
        let line = geo::LineString::from(vec![(0., 0.), (1., 0.)]);
        let geometries = [
            geo::Geometry::Line(geo::Line::new((0., 0.), (1., 0.))),
            geo::Geometry::LineString(line.clone()),
            geo::Geometry::MultiLineString(geo::MultiLineString(vec![line.clone(), line.clone()])),
            geo::Geometry::GeometryCollection(geo::GeometryCollection(vec![
                geo::Geometry::LineString(line.clone()),
                geo::Geometry::Point(geo::Point::new(5., 5.)),
            ])),
            geo::Geometry::Point(geo::Point::new(1., 1.)),
        ];
        let mut lines = Vec::new();
        for g in &geometries {
            extract_lines(g, &mut lines);
        }
        assert_eq!(lines, vec![line; 5]);
    }

    #[test]
    fn output_write_and_flush_failures_have_portable_exit_statuses() {
        for json in [false, true] {
            for validation in [false, true] {
                for kind in [io::ErrorKind::BrokenPipe, io::ErrorKind::Other] {
                    for (limit, flush_fails) in [(Some(0), false), (Some(17), false), (None, true)]
                    {
                        let mut output = FailingWriter {
                            kind,
                            write_limit: limit,
                            flush_fails,
                            flushed: false,
                        };
                        let mut diagnostics = Vec::new();
                        let result = run(&matches(json, validation), &mut output, &mut diagnostics);
                        assert!(output.flushed);
                        let code = exit_status(result, json, &mut diagnostics);
                        assert_eq!(
                            code,
                            if kind == io::ErrorKind::BrokenPipe {
                                0
                            } else {
                                2
                            }
                        );
                        if kind == io::ErrorKind::BrokenPipe {
                            assert!(diagnostics.is_empty());
                        } else if json {
                            let error: serde_json::Value =
                                serde_json::from_slice(&diagnostics).unwrap();
                            assert!(error["error"].is_string());
                        } else {
                            assert!(diagnostics.starts_with(b"error: "));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn complete_reports_keep_status_newline_and_json_format() {
        for json in [false, true] {
            for validation in [false, true] {
                let mut output = Vec::new();
                let code = run(&matches(json, validation), &mut output, &mut Vec::new()).unwrap();
                assert_eq!(code, u8::from(validation));
                assert_eq!(output.last(), Some(&b'\n'));
                if json {
                    assert!(serde_json::from_slice::<serde_json::Value>(&output).is_ok());
                }
            }
        }
    }

    #[test]
    fn broken_pipe_json_errors_are_recognized_through_context() {
        let mut output = FailingWriter {
            kind: io::ErrorKind::BrokenPipe,
            write_limit: Some(0),
            flush_fails: false,
            flushed: false,
        };
        let error = report::json(&mut output, &Rule::ALL).unwrap_err();
        assert!(error.downcast_ref::<serde_json::Error>().is_some());
        assert!(is_broken_pipe(&error.context("serializing report")));
    }

    #[test]
    fn unavailable_stderr_does_not_panic_for_warnings_or_errors() {
        let mut diagnostics = FailingWriter {
            kind: io::ErrorKind::BrokenPipe,
            write_limit: Some(0),
            flush_fails: true,
            flushed: false,
        };
        assert_eq!(
            exit_status(
                Err(anyhow::anyhow!("operational failure")),
                false,
                &mut diagnostics
            ),
            2
        );
        assert_eq!(
            exit_status(
                Err(anyhow::anyhow!("operational failure")),
                true,
                &mut diagnostics
            ),
            2
        );
        let report = topology_checker_io::report_store::DiskReport::validate(
            &Topology::new(0.).unwrap(),
            None,
        )
        .unwrap();
        let warnings = ["test warning".to_string()];
        for json in [false, true] {
            report::print(
                &report,
                &warnings,
                json,
                false,
                &mut Vec::new(),
                &mut diagnostics,
            )
            .unwrap();
        }
    }
}
