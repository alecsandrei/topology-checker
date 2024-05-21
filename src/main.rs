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
        /// GDAL driver to use for output files.
        pub gdal_driver: Option<String>,
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
        Interactive { output: Option<PathBuf> },
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
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Optional geometry to check against. By default compares
            /// against other features in the input.
            other: Option<PathBuf>,
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
            /// Whether or not to check for self overlaps.
            /// This can't be true if 'other' has been specified.
            #[arg(long, short, action)]
            self_overlap: Option<bool>,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Optional geometry to check against. By default compares to itself
            other: Option<PathBuf>,
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
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Optional geometry to check against. By default compares to itself
            other: Option<PathBuf>,
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

use args::{
    Command, Drivers, GeometryRules, LineRules, PointRules, PolygonRules, TopologyCheckerArgs,
    Utilities,
};
use clap::Parser;
use colored::Colorize;
use gdal::{vector::ToGdal, LayerOptions};
use itertools::Itertools;
use rayon::iter::{ParallelBridge, ParallelIterator};
use regex::Regex;
use serde::Deserialize;
use topology_checker::{
    algorithm::merge_linestrings,
    rule::{
        MustNotBeMultipart, MustNotHaveDangles, MustNotIntersect, MustNotOverlap,
        MustNotSelfOverlap,
    },
    util::{
        explode_linestrings, flatten_linestrings, flatten_points, flatten_polygons,
        geometries_to_file, GdalDrivers,
    },
    GeometryError, TopologyResult, TopologyResults, VectorDataset,
};

fn main() {
    let args = TopologyCheckerArgs::parse();
    match args.command {
        Command::Interactive { .. } => interactive_mode(args),
        Command::Geometry(_) | Command::Line(_) | Command::Point(_) | Command::Polygon(_) => {
            parse_rules(args);
        }
        Command::Utilities(_) | Command::GdalDrivers(_) => parse_utils(args),
    }
}

/// Used to get the serialized rule name from a [Command] object.
/// TODO: Find a better solution for this, it's really ugly.
fn rule_name(command: &Command) -> String {
    let value = serde_json::to_value(command).unwrap();
    let geometry = value
        .as_object()
        .unwrap()
        .into_iter()
        .map(|x| x.0)
        .next()
        .unwrap();
    value
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
        .clone()
}

fn interactive_mode(args: TopologyCheckerArgs) {
    println!("{}", "Write 'summary' to stop the loop.".yellow());
    let mut commands: Vec<Command> = Vec::new();
    'outer: loop {
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .expect("Wrong input.");
        if input.trim_end() == "summary" {
            println!("{:?}", commands);
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
        let re = Regex::new(r#"([-\w]+)=("[^"]+"|\S+)"#).unwrap();
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
                subcaptures.next().unwrap().unwrap().as_str(),
                subcaptures
                    .next()
                    .unwrap()
                    .unwrap()
                    .as_str()
                    .replace("\"", ""),
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
                    println!("Command successfully added.")
                }
            }
            Err(error) => eprintln!("{}", error.to_string().red()),
        }
    }
    let results = TopologyResults(
        commands
            .into_iter()
            .par_bridge()
            .map(|command| {
                let args = TopologyCheckerArgs {
                    gdal_driver: args.gdal_driver.clone(),
                    command: command,
                };
                (rule_name(&args.command), parse_rules(args).unwrap())
            })
            .collect(),
    );
    results.summary(None);
}

/// If the output location is provided, the [TopologyResult] gets consumed and
/// the geometries are saved at that specific location, returning None.
/// Otherwise, Some(TopologyResult) is returned.
fn parse_rules(args: TopologyCheckerArgs) -> Option<TopologyResult<f64>> {
    match args.command {
        Command::GdalDrivers(_) | Command::Interactive { .. } | Command::Utilities(_) => {
            unreachable!()
        }
        Command::Point(command) => match command.command {
            PointRules::MustNotOverlap {
                points,
                overlaps,
                other,
            } => {
                let vector_dataset = VectorDataset::new(&points);
                let points = flatten_points(vector_dataset.to_geo().unwrap());
                if let Some(other) = other {
                    let other = flatten_points(VectorDataset::new(&other).to_geo().unwrap());
                    let result = points.must_not_overlap_with(other);
                    if !result.is_valid() {
                        if let Some(overlaps) = overlaps {
                            result.unwrap_err_point().to_file(
                                &overlaps,
                                args.gdal_driver,
                                Some(LayerOptions {
                                    name: "overlaps",
                                    srs: vector_dataset.crs().as_ref(),
                                    ..Default::default()
                                }),
                            );
                        }
                        return None;
                    }
                    Some(result)
                } else {
                    let result = points.must_not_overlap();
                    if !result.is_valid() {
                        if let Some(overlaps) = overlaps {
                            result.unwrap_err_point().to_file(
                                &overlaps,
                                args.gdal_driver,
                                Some(LayerOptions {
                                    name: "overlaps",
                                    srs: vector_dataset.crs().as_ref(),
                                    ..Default::default()
                                }),
                            );
                            return None;
                        }
                    }
                    Some(result)
                }
            }
        },
        Command::Line(command) => match command.command {
            LineRules::MustNotHaveDangles { lines, dangles } => {
                let vector_dataset = VectorDataset::new(&lines);
                let lines = vector_dataset.to_geo().unwrap();
                let lines = flatten_linestrings(lines);
                let result = lines.must_not_have_dangles();
                if !result.is_valid() {
                    if let Some(dangles) = dangles {
                        result.unwrap_err_point().to_file(
                            &dangles,
                            args.gdal_driver,
                            Some(LayerOptions {
                                name: "dangles",
                                srs: vector_dataset.crs().as_ref(),
                                ..Default::default()
                            }),
                        );
                        return None;
                    }
                }
                Some(result)
            }
            LineRules::MustNotIntersect {
                lines,
                single_points,
                collinear_lines,
            } => {
                let vector_dataset = VectorDataset::new(&lines);
                let lines = vector_dataset.to_geo().unwrap();
                let lines = flatten_linestrings(lines);
                let result = lines.must_not_intersect();
                if !result.is_valid() {
                    if single_points.is_some() | collinear_lines.is_some() {
                        for error in result.unwrap_err().into_iter() {
                            if let GeometryError::Point(_) = error {
                                if let Some(single_points) = single_points.clone() {
                                    error.to_file(
                                        &single_points,
                                        args.gdal_driver.clone(),
                                        Some(LayerOptions {
                                            name: "intersections",
                                            srs: vector_dataset.crs().as_ref(),
                                            ..Default::default()
                                        }),
                                    );
                                }
                            } else if let GeometryError::LineString(_) = error {
                                if let Some(collinear_lines) = collinear_lines.clone() {
                                    error.to_file(
                                        &collinear_lines,
                                        args.gdal_driver.clone(),
                                        Some(LayerOptions {
                                            name: "intersections",
                                            srs: vector_dataset.crs().as_ref(),
                                            ..Default::default()
                                        }),
                                    );
                                }
                            }
                        }
                        return None;
                    }
                }
                Some(result)
            }
            LineRules::MustNotOverlap {
                lines,
                overlaps,
                self_overlap,
                other,
            } => {
                let vector_dataset = VectorDataset::new(&lines);
                let lines = vector_dataset.to_geo().unwrap();
                let lines = flatten_linestrings(lines);
                if let Some(other) = other {
                    let other = flatten_linestrings(VectorDataset::new(&other).to_geo().unwrap());
                    let result = lines.must_not_overlap_with(other);
                    if !result.is_valid() {
                        if let Some(overlaps) = overlaps {
                            result.unwrap_err_linestring().to_file(
                                &overlaps,
                                args.gdal_driver.clone(),
                                Some(LayerOptions {
                                    name: "overlaps",
                                    srs: vector_dataset.crs().as_ref(),
                                    ..Default::default()
                                }),
                            );
                            return None;
                        }
                    }
                    Some(result)
                } else {
                    let result;
                    if self_overlap.is_some() && self_overlap.unwrap() {
                        result = lines.must_not_self_overlap();
                    } else {
                        result = lines.must_not_overlap();
                    }
                    if !result.is_valid() {
                        if let Some(overlaps) = overlaps {
                            result.unwrap_err_linestring().to_file(
                                &overlaps,
                                args.gdal_driver.clone(),
                                Some(LayerOptions {
                                    name: "overlaps",
                                    srs: vector_dataset.crs().as_ref(),
                                    ..Default::default()
                                }),
                            );
                            return None;
                        }
                    }
                    Some(result)
                }
            }
        },
        Command::Polygon(command) => match command.command {
            PolygonRules::MustNotOverlap {
                polygons,
                overlaps,
                other,
            } => {
                let vector_dataset = VectorDataset::new(&polygons);
                let polygons = vector_dataset.to_geo().unwrap();
                let polygons = flatten_polygons(polygons);
                if let Some(other) = other {
                    let other_polygons = VectorDataset::new(&other).to_geo().unwrap();
                    let other_polygons = flatten_polygons(other_polygons);
                    let result = polygons.must_not_overlap_with(other_polygons);
                    if !result.is_valid() {
                        if let Some(overlaps) = overlaps {
                            result.unwrap_err_polygon().to_file(
                                &overlaps,
                                args.gdal_driver,
                                Some(LayerOptions {
                                    name: "overlaps",
                                    srs: vector_dataset.crs().as_ref(),
                                    ..Default::default()
                                }),
                            );
                            return None;
                        }
                    }
                    Some(result)
                } else {
                    let result = polygons.must_not_overlap();
                    if !result.is_valid() {
                        println!("No errors found.")
                    } else {
                        if let Some(overlaps) = overlaps {
                            result.unwrap_err_polygon().to_file(
                                &overlaps,
                                args.gdal_driver,
                                Some(LayerOptions {
                                    name: "overlaps",
                                    srs: vector_dataset.crs().as_ref(),
                                    ..Default::default()
                                }),
                            );
                            return None;
                        }
                    }
                    Some(result)
                }
            }
        },
        Command::Geometry(command) => match command.command {
            GeometryRules::MustNotBeMultipart {
                geometries,
                multiparts,
            } => {
                let dataset = VectorDataset::new(&geometries);
                let geometry = dataset.to_geo().unwrap();
                let result = geometry.must_not_be_multipart();
                if !result.is_valid() {
                    if let Some(multiparts) = multiparts {
                        result.unwrap_err().into_iter().next().unwrap().to_file(
                            &multiparts,
                            args.gdal_driver,
                            Some(LayerOptions {
                                name: "multiparts",
                                srs: dataset.crs().as_ref(),
                                ..Default::default()
                            }),
                        );
                        return None;
                    }
                }
                Some(result)
            }
        },
    }
}

fn parse_utils(args: TopologyCheckerArgs) {
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
                let dataset = VectorDataset::new(&linestrings);
                let geometry = dataset.to_geo().unwrap();
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
                        srs: dataset.crs().as_ref(),
                        ..Default::default()
                    }),
                )
            }
            Utilities::MergeLinestrings {
                linestrings,
                merged,
            } => {
                let dataset = VectorDataset::new(&linestrings);
                let geometry = dataset.to_geo().unwrap();
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
                        srs: dataset.crs().as_ref(),
                        ..Default::default()
                    }),
                )
            }
        },
    }
}
