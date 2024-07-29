mod args {
    use clap::{Args, Parser, Subcommand};
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;

    /// Parse a single key-value pair
    fn parse_key_val<T, U>(s: &str) -> Result<U, Box<dyn std::error::Error + Send + Sync + 'static>>
    where
        T: std::str::FromStr,
        T::Err: std::error::Error + Send + Sync + 'static,
        U: std::str::FromStr,
        U::Err: std::error::Error + Send + Sync + 'static,
    {
        let pos = s
            .find('=')
            .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{s}`"))?;
        Ok(s[pos + 1..].parse()?)
    }

    #[derive(Debug, Parser)]
    #[clap(author, version, about)]
    pub struct TopologyCheckerArgs {
        #[clap(long, env)]
        /// GDAL driver to use for the exported datasets.
        pub gdal_driver: Option<String>,
        #[clap(long, short, action)]
        /// Only use GDAL for reading.
        pub use_gdal: bool,
        #[clap(long, env)]
        /// EPSG code
        pub epsg: Option<u32>,
        #[clap(long, short, action)]
        /// Print elapsed time.
        pub elapsed: bool,
        #[clap(subcommand)]
        pub command: Command,
    }

    #[derive(Debug, Serialize, PartialEq, Deserialize, Subcommand)]
    #[serde(rename_all = "lowercase")]
    pub enum Command {
        /// Topology checks for point geometries
        Point(PointCommand),
        /// Topology checks for line geometries
        Line(LineCommand),
        /// Topology checks for polygon geometries
        Polygon(PolygonCommand),
        /// Topology checks for any geometry type
        Geometry(GeometryCommand),
        /// Print the allowed GDAL drivers
        GdalDrivers(GdalDriversCommand),
        /// Extra vector data utilities
        Utilities(UtilitiesCommand),
        /// Interactive mode
        Interactive {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            output: PathBuf,
        },
    }

    #[derive(Debug, PartialEq, Args, Serialize, Deserialize)]
    pub struct PointCommand {
        #[clap(subcommand)]
        pub command: PointRules,
    }

    #[derive(Debug, PartialEq, Args, Serialize, Deserialize)]
    pub struct LineCommand {
        #[clap(subcommand)]
        pub command: LineRules,
    }

    #[derive(Debug, PartialEq, Args, Serialize, Deserialize)]
    pub struct PolygonCommand {
        #[clap(subcommand)]
        pub command: PolygonRules,
    }

    #[derive(Debug, PartialEq, Args, Serialize, Deserialize)]
    pub struct GeometryCommand {
        #[clap(subcommand)]
        pub command: GeometryRules,
    }

    #[derive(Debug, PartialEq, Args, Serialize, Deserialize)]
    pub struct GdalDriversCommand {
        #[clap(subcommand)]
        pub command: Drivers,
    }

    #[derive(Debug, Subcommand, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum PointRules {
        #[command(arg_required_else_help(true))]
        MustNotOverlap {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            points: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: Option<PathBuf>,
        },
        #[command(arg_required_else_help(true))]
        MustNotOverlapWith {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input points
            points: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input points to check against
            other: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: Option<PathBuf>,
        },
        #[command(arg_required_else_help(true))]
        MustBeInside {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input points
            points: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input polygons to check against
            polygons: PathBuf,
            /// The outside points
            outside: Option<PathBuf>,
        },
    }

    #[derive(Debug, Subcommand, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum LineRules {
        #[command(arg_required_else_help(true))]
        MustNotOverlap {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            lines: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: Option<PathBuf>,
        },
        #[command(arg_required_else_help(true))]
        MustNotOverlapWith {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            lines: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines to check against
            other: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: Option<PathBuf>,
        },
        #[command(arg_required_else_help(true))]
        MustNotSelfOverlap {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            lines: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: Option<PathBuf>,
        },
        #[command(arg_required_else_help(true))]
        MustNotHaveDangles {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            lines: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Output dangles
            dangles: Option<PathBuf>,
        },
        #[command(arg_required_else_help(true))]
        MustNotIntersect {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            lines: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Output point intersections
            single_points: Option<PathBuf>,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Output line intersections
            collinear_lines: Option<PathBuf>,
        },
        #[command(arg_required_else_help(true))]
        MustBeInside {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            lines: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input polygons
            polygons: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Output outside lines
            outside_lines: Option<PathBuf>,
        },
    }

    #[derive(Debug, Subcommand, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum PolygonRules {
        #[command(arg_required_else_help(true))]
        MustNotOverlap {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input polygons
            polygons: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: Option<PathBuf>,
        },
        #[command(arg_required_else_help(true))]
        MustNotOverlapWith {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input polygons
            polygons: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input polygons to check against
            other: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: Option<PathBuf>,
        },
        #[command(arg_required_else_help(true))]
        MustNotHaveGaps {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input polygons
            polygons: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Output gaps
            gaps: Option<PathBuf>,
        },
    }

    #[derive(Debug, Subcommand, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum GeometryRules {
        #[command(arg_required_else_help(true))]
        MustNotBeMultipart {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input geometries
            geometries: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output multipart geometries
            multiparts: Option<PathBuf>,
        },
    }

    #[derive(Debug, Subcommand, PartialEq, Serialize, Deserialize)]
    pub enum Drivers {
        /// List readable drivers
        Read,
        /// List writeable drivers
        Write,
        /// List readable and writeable drivers
        ReadAndWrite,
    }

    #[derive(Debug, PartialEq, Args, Serialize, Deserialize)]
    pub struct UtilitiesCommand {
        #[clap(subcommand)]
        pub command: Utilities,
    }

    #[derive(Debug, Subcommand, PartialEq, Serialize, Deserialize)]
    pub enum Utilities {
        /// Merge linestrings
        #[command(arg_required_else_help(true))]
        MergeLinestrings {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The input linestrings
            linestrings: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output merged linestrings
            merged: PathBuf,
        },
        /// Explode linestrings
        #[command(arg_required_else_help(true))]
        ExplodeLinestrings {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The input linestrings
            linestrings: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output exploded lines
            lines: PathBuf,
        },
    }
}

use anyhow::Context;
use args::{
    Command, Drivers, GeometryRules, LineRules, PointRules, PolygonRules, TopologyCheckerArgs,
    Utilities,
};
use clap::Parser;
use colored::Colorize;
use gdal::{vector::ToGdal, LayerOptions};
use rayon::{iter::ParallelBridge, iter::ParallelIterator};
use regex::Regex;
use serde::Deserialize;
use topology_checker::{
    algorithm::merge_linestrings,
    rule::{
        MustBeInside, MustNotBeMultipart, MustNotHaveDangles, MustNotHaveGaps, MustNotIntersect,
        MustNotOverlap, MustNotSelfOverlap,
    },
    util::{
        explode_linestrings, flatten_linestrings, flatten_points, flatten_polygons,
        geometries_to_file, validate_srs, GdalDrivers,
    },
    ExportConfig, TopologyError, TopologyResult, TopologyResults, VectorDataset,
};
#[cfg(windows)]
fn enable_colors_for_windows() {
    // The result of the set_virtual_terminal function is always Ok(())
    // thus it is ok to unwrap
    colored::control::set_virtual_terminal(true).unwrap();
}

#[cfg(not(windows))]
fn enable_colors_for_windows() {}

fn main() -> anyhow::Result<()> {
    enable_colors_for_windows();
    let args = TopologyCheckerArgs::parse();
    let mut start = None;
    if args.elapsed {
        let time = std::time::Instant::now();
        start.replace(time);
    }
    match args.command {
        Command::Interactive { .. } => interactive_mode(args)?,
        Command::Geometry(_) | Command::Line(_) | Command::Point(_) | Command::Polygon(_) => {
            parse_rules(args, true)?;
        }
        Command::Utilities(_) | Command::GdalDrivers(_) => parse_utils(args)?,
    }
    if let Some(start) = start {
        println!("Elapsed time: {:?}", start.elapsed());
    }
    Ok(())
}

type RuleName = String;

/// Used to get the serialized rule name from a [Command] object.
/// TODO: Find a better solution for this, it's really ugly.
fn rule_name(command: &Command) -> anyhow::Result<RuleName> {
    let value = serde_json::to_value(command)?;
    let geometry = value
        .as_object()
        .unwrap()
        .into_iter()
        .map(|x| x.0)
        .next()
        .unwrap();
    let rule_name = value
        .as_object()
        .unwrap()
        .get(geometry)
        .unwrap()
        .as_object()
        .unwrap()
        .into_iter()
        .map(|x| x.1.as_object().unwrap().keys().next().unwrap())
        .next()
        .unwrap()
        .clone();
    Ok(rule_name)
}

fn interactive_mode(args: TopologyCheckerArgs) -> anyhow::Result<()> {
    println!("{}", "Write 'summary' to stop the loop. Example input: line must-not-have-dangles lines=./lines.shp".yellow());
    let mut commands: Vec<Command> = Vec::new();
    'outer: loop {
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .with_context(|| "Wrong input line, please try again.")?;
        if input.trim_end() == "summary" {
            break;
        }
        let mut slices = input.split_whitespace();
        let geometry = match slices.next() {
            Some(geometry) => {
                match geometry {
                    "point" | "geometry" | "polygon" | "line" => geometry,
                    _ => {
                        eprintln!("{} not allowed. Choose between 'point', 'geometry', 'polygon' and 'line'", geometry.red());
                        continue;
                    }
                }
            }
            None => {
                eprintln!("{}", "Geometry parameter was not provided.".red());
                continue;
            }
        };
        let rule = match slices.next() {
            Some(rule) => rule,
            None => {
                eprintln!("{}", "Rule parameter was not provided.".red());
                continue;
            }
        };
        let mut args = std::collections::HashMap::new();
        // TODO: document what this regex does.
        let re = Regex::new(r#"([-\w]+)=("[^"]+"|\S+)"#)?;
        for captures in re.captures_iter(&input) {
            let mut subcaptures: std::iter::Skip<regex::SubCaptureMatches> =
                captures.iter().skip(1);
            if subcaptures.len() != 2 {
                eprintln!(
                    "Problem near {:?}. Failed to parse.",
                    subcaptures.next().unwrap().unwrap()
                );
                continue 'outer;
            }
            args.insert(
                subcaptures
                    .next()
                    .unwrap()
                    .unwrap()
                    .as_str()
                    .replace("--", "")
                    .replace("-", "_"),
                subcaptures
                    .next()
                    .unwrap()
                    .unwrap()
                    .as_str()
                    .replace("\"", ""), // remove quotation marks
            );
        }

        let json = serde_json::json!({
            geometry: {
                "command": {rule: args}
            }
        });
        let deserialized = Command::deserialize(json);
        match deserialized {
            Ok(deserialized) => {
                if commands.contains(&deserialized) {
                    eprintln!("{}", "The command was already added".red())
                } else {
                    commands.push(deserialized);
                    println!(
                        "{}",
                        format!("Command {} successfully added.", commands.len()).green()
                    )
                }
            }
            Err(error) => eprintln!("{}", error.to_string().red()),
        }
    }
    // Result implements FromIterator and thus we can move it outside
    let results: anyhow::Result<Vec<_>> = commands
        .into_iter()
        .enumerate()
        .par_bridge()
        .map(
            |(mut index, command)| -> anyhow::Result<(String, TopologyResult<_>)> {
                index += 1;
                let args = TopologyCheckerArgs {
                    gdal_driver: args.gdal_driver.clone(),
                    use_gdal: args.use_gdal,
                    epsg: args.epsg,
                    elapsed: args.elapsed,
                    command: command,
                };
                let rule_name = format!("{}-{}", index, rule_name(&args.command)?);
                Ok((rule_name, parse_rules(args, false)?))
            },
        )
        .collect();
    let topology_results = TopologyResults::new(results?);
    match args.command {
        Command::Interactive { output } => topology_results.export(&output, args.epsg)?,
        _ => unreachable!(),
    }
    Ok(())
}

fn parse_rules(args: TopologyCheckerArgs, summarize: bool) -> anyhow::Result<TopologyResult<f64>> {
    let rule_name = rule_name(&args.command)?;
    let options = LayerOptions {
        name: &rule_name.clone(),
        ..Default::default()
    };
    let mut config = ExportConfig {
        rule_name: rule_name.clone(),
        options: options,
        ..Default::default()
    };
    let result = match args.command {
        Command::Point(ref command) => match &command.command {
            PointRules::MustNotOverlap { points, overlaps } => {
                let mut vector_dataset = VectorDataset::new(&points, args.use_gdal)?;
                let points = flatten_points(vector_dataset.to_geo()?);
                let srs = vector_dataset.srs()?;
                let result = points.must_not_overlap();
                if overlaps.is_some() && !result.is_valid() {
                    config.output = overlaps.as_ref();
                    config.options.srs = srs.as_ref();
                    result.unwrap_err_point().export(config)?
                }
                result
            }
            PointRules::MustNotOverlapWith {
                points,
                other,
                overlaps,
            } => {
                let mut vector_dataset = VectorDataset::new(&points, args.use_gdal)?;
                let mut other = VectorDataset::new(&other, args.use_gdal)?;
                validate_srs(&vector_dataset, &other)?;
                let other = flatten_points(other.to_geo()?);
                let points = flatten_points(vector_dataset.to_geo()?);
                let srs = vector_dataset.srs()?;
                let result = points.must_not_overlap_with(other);
                if overlaps.is_some() && !result.is_valid() {
                    config.output = overlaps.as_ref();
                    config.options.srs = srs.as_ref();
                    result.unwrap_err_point().export(config)?
                }
                result
            }
            PointRules::MustBeInside {
                points,
                polygons,
                outside,
            } => {
                let mut vector_dataset = VectorDataset::new(&points, args.use_gdal)?;
                let mut other = VectorDataset::new(&polygons, args.use_gdal)?;
                validate_srs(&vector_dataset, &other)?;
                let other = other.to_geo()?;
                let other = flatten_polygons(other);
                let geometries = vector_dataset.to_geo()?;
                let points = flatten_points(geometries);
                let srs = vector_dataset.srs()?;
                let result = points.must_be_inside(other);
                if outside.is_some() && !result.is_valid() {
                    config.output = outside.as_ref();
                    config.options.srs = srs.as_ref();
                    result.unwrap_err_point().export(config)?
                }
                result
            }
        },
        Command::Line(command) => match command.command {
            LineRules::MustNotHaveDangles { lines, dangles } => {
                let mut vector_dataset = VectorDataset::new(&lines, args.use_gdal)?;
                let srs = vector_dataset.srs()?;
                let lines = vector_dataset.to_geo()?;
                let lines = flatten_linestrings(lines);
                let result = lines.must_not_have_dangles();
                if dangles.is_some() && !result.is_valid() {
                    config.output = dangles.as_ref();
                    config.options.srs = srs.as_ref();
                    result.unwrap_err_point().export(config)?;
                };
                result
            }
            LineRules::MustNotIntersect {
                lines,
                single_points,
                collinear_lines,
            } => {
                let mut vector_dataset = VectorDataset::new(&lines, args.use_gdal)?;
                let srs = vector_dataset.srs()?;
                let lines = vector_dataset.to_geo()?;
                let lines = flatten_linestrings(lines);
                let result = lines.must_not_intersect();
                // Some workaround for the case where the rule can have
                // two output files.
                if (single_points.is_some() | collinear_lines.is_some()) && !result.is_valid() {
                    config.options.srs = srs.as_ref();
                    for error in result.unwrap_err() {
                        if let TopologyError::Point(_) = error {
                            if let Some(ref single_points) = single_points {
                                let mut config = config.clone();
                                config.output = Some(single_points);
                                error.export(config)?
                            }
                        }
                        if let TopologyError::LineString(_) = error {
                            if let Some(ref collinear_lines) = collinear_lines {
                                let mut config = config.clone();
                                config.output = Some(collinear_lines);
                                error.export(config)?
                            }
                        }
                    }
                }
                result
            }
            LineRules::MustNotOverlap { lines, overlaps } => {
                let mut vector_dataset = VectorDataset::new(&lines, args.use_gdal)?;
                let srs = vector_dataset.srs()?;
                let lines = vector_dataset.to_geo()?;
                let lines = flatten_linestrings(lines);
                let result = lines.must_not_overlap();
                if overlaps.is_some() && !result.is_valid() {
                    config.options.srs = srs.as_ref();
                    config.output = overlaps.as_ref();
                    result.unwrap_err_linestring().export(config)?
                }
                result
            }
            LineRules::MustNotOverlapWith {
                lines,
                other,
                overlaps,
            } => {
                let mut vector_dataset = VectorDataset::new(&lines, args.use_gdal)?;
                let mut other = VectorDataset::new(&other, args.use_gdal)?;
                validate_srs(&vector_dataset, &other)?;
                let srs = vector_dataset.srs()?;
                let lines = vector_dataset.to_geo()?;
                let lines = flatten_linestrings(lines);
                let other = flatten_linestrings(other.to_geo()?);
                let result = lines.must_not_overlap_with(other);
                if overlaps.is_some() && !result.is_valid() {
                    config.options.srs = srs.as_ref();
                    config.output = overlaps.as_ref();
                    result.unwrap_err_linestring().export(config)?;
                }
                result
            }
            LineRules::MustNotSelfOverlap { lines, overlaps } => {
                let mut vector_dataset = VectorDataset::new(&lines, args.use_gdal)?;
                let srs = vector_dataset.srs()?;
                let lines = vector_dataset.to_geo()?;
                let lines = flatten_linestrings(lines);
                let result = lines.must_not_self_overlap();
                if overlaps.is_some() && !result.is_valid() {
                    config.options.srs = srs.as_ref();
                    config.output = overlaps.as_ref();
                    result.unwrap_err_linestring().export(config)?
                }
                result
            }
            LineRules::MustBeInside {
                lines,
                polygons,
                outside_lines,
            } => {
                let mut vector_dataset = VectorDataset::new(&lines, args.use_gdal)?;
                let mut other = VectorDataset::new(&polygons, args.use_gdal)?;
                validate_srs(&vector_dataset, &other)?;
                let srs = vector_dataset.srs()?;
                let lines = vector_dataset.to_geo()?;
                let lines = flatten_linestrings(lines);
                let polygons = flatten_polygons(other.to_geo()?);
                let result = lines.must_be_inside(polygons);
                if outside_lines.is_some() && !result.is_valid() {
                    config.options.srs = srs.as_ref();
                    config.output = outside_lines.as_ref();
                    result.unwrap_err_linestring().export(config)?;
                }
                result
            }
        },
        Command::Polygon(command) => match command.command {
            PolygonRules::MustNotOverlap { polygons, overlaps } => {
                let mut vector_dataset = VectorDataset::new(&polygons, args.use_gdal)?;
                let srs = vector_dataset.srs()?;
                let polygons = vector_dataset.to_geo()?;
                let polygons = flatten_polygons(polygons);
                let result = polygons.must_not_overlap();
                if overlaps.is_some() && !result.is_valid() {
                    config.output = overlaps.as_ref();
                    config.options.srs = srs.as_ref();
                    result.unwrap_err_polygon().export(config)?;
                }
                result
            }
            PolygonRules::MustNotOverlapWith {
                polygons,
                other,
                overlaps,
            } => {
                let mut vector_dataset = VectorDataset::new(&polygons, args.use_gdal)?;
                let mut other = VectorDataset::new(&other, args.use_gdal)?;
                let srs = vector_dataset.srs()?;
                let polygons = vector_dataset.to_geo()?;
                let polygons = flatten_polygons(polygons);
                let other = flatten_polygons(other.to_geo()?);
                let result = polygons.must_not_overlap_with(other);
                if overlaps.is_some() {
                    config.output = overlaps.as_ref();
                    config.options.srs = srs.as_ref();
                    result.unwrap_err_polygon().export(config)?;
                }
                result
            }
            PolygonRules::MustNotHaveGaps { polygons, gaps } => {
                let mut vector_dataset = VectorDataset::new(&polygons, args.use_gdal)?;
                let srs = vector_dataset.srs()?;
                let polygons = vector_dataset.to_geo()?;
                let polygons = flatten_polygons(polygons);
                let result = polygons.must_not_have_gaps();
                if gaps.is_some() {
                    config.output = gaps.as_ref();
                    config.options.srs = srs.as_ref();
                    result.unwrap_err_linestring().export(config)?;
                }
                result
            }
        },
        Command::Geometry(command) => match command.command {
            GeometryRules::MustNotBeMultipart {
                geometries,
                multiparts,
            } => {
                let mut dataset = VectorDataset::new(&geometries, args.use_gdal)?;
                let srs = dataset.srs()?;
                let geometry = dataset.to_geo()?;
                let result = geometry.must_not_be_multipart();
                if multiparts.is_some() {
                    config.options.srs = srs.as_ref();
                    config.output = multiparts.as_ref();
                    for error in result.unwrap_err() {
                        error.export(config.clone())?;
                    }
                }
                result
            }
        },
        Command::GdalDrivers(_) | Command::Interactive { .. } | Command::Utilities(_) => {
            unreachable!()
        }
    };
    if summarize {
        result.summary(Some(rule_name));
    }
    Ok(result)
}

fn parse_utils(args: TopologyCheckerArgs) -> anyhow::Result<()> {
    match args.command {
        Command::Geometry(_)
        | Command::Line(_)
        | Command::Point(_)
        | Command::Interactive { .. }
        | Command::Polygon(_) => {
            unreachable!()
        }
        Command::GdalDrivers(command) => match command.command {
            Drivers::Read => {
                for (driver, extension) in GdalDrivers.read() {
                    println!("{}: {}", driver, extension)
                }
            }
            Drivers::Write => {
                for (driver, extension) in GdalDrivers.write() {
                    println!("{}: {}", driver, extension)
                }
            }
            Drivers::ReadAndWrite => {
                for (driver, extension) in GdalDrivers.read_write() {
                    println!("{}: {}", driver, extension)
                }
            }
        },
        Command::Utilities(command) => match command.command {
            Utilities::ExplodeLinestrings { linestrings, lines } => {
                let mut dataset = VectorDataset::new(&linestrings, args.use_gdal)?;
                let geometry = dataset.to_geo()?;
                let linestrings = flatten_linestrings(geometry);
                let exploded = explode_linestrings(&linestrings);
                geometries_to_file(
                    exploded
                        .into_iter()
                        .map(|line| line.to_gdal().expect("Failed to convert to GDAL."))
                        .collect(),
                    &lines,
                    args.gdal_driver,
                    Some(LayerOptions {
                        name: "merged_linestrings",
                        srs: dataset.srs()?.as_ref(),
                        ..Default::default()
                    }),
                )
            }
            Utilities::MergeLinestrings {
                linestrings,
                merged,
            } => {
                let mut dataset = VectorDataset::new(&linestrings, args.use_gdal)?;
                let geometry = dataset.to_geo()?;
                let linestrings = flatten_linestrings(geometry);
                let merged_linestrings = merge_linestrings(linestrings);
                geometries_to_file(
                    merged_linestrings
                        .into_iter()
                        .map(|line| line.to_gdal().expect("Failed to convert to GDAL."))
                        .collect(),
                    &merged,
                    args.gdal_driver,
                    Some(LayerOptions {
                        name: "merged_linestrings",
                        srs: dataset.srs()?.as_ref(),
                        ..Default::default()
                    }),
                )
            }
        },
    }
    Ok(())
}
