mod args {
    use clap::{Args, Parser, Subcommand};
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
        pub command: Commands,
    }

    #[derive(Debug, Subcommand)]
    pub enum Commands {
        #[command(arg_required_else_help(true))]
        MustNotHaveDangles {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            lines: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Output dangles
            dangles: PathBuf,
        },
        #[command(arg_required_else_help(true))]
        MustNotIntersect {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            lines: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Output point intersections
            single_points: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Output line intersections
            collinear_lines: PathBuf,
        },
        #[command(arg_required_else_help(true))]
        MustNotOverlap {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The input geometry
            geometry: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Optional geometry to check against. By default compares to itself
            other: Option<PathBuf>,
        },
        /// Print the allowed GDAL drivers
        GdalDrivers(GdalDriversCommand),
        /// Extra vector data utilities
        Utilities(UtilitiesCommand),
    }

    #[derive(Debug, Args)]
    pub struct GdalDriversCommand {
        #[clap(subcommand)]
        pub command: Drivers,
    }

    #[derive(Debug, Subcommand)]
    pub enum Drivers {
        /// List readable drivers
        Read,
        /// List writeable drivers
        Write,
        /// List readable and writeable drivers
        ReadAndWrite,
    }

    #[derive(Debug, Args)]
    pub struct UtilitiesCommand {
        #[clap(subcommand)]
        pub command: Utilities,
    }

    #[derive(Debug, Subcommand)]
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

use args::{Commands, Drivers, TopologyCheckerArgs, Utilities};
use clap::Parser;
use gdal::LayerOptions;
use std::time;
use topology_checker::{
    algorithm::lines_to_linestring,
    rule::{MustNotHaveDangles, MustNotIntersect, MustNotOverlap},
    util::{flatten_lines, flatten_linestrings, flatten_polygons, geometries_to_file, GdalDrivers},
    VectorDataset,
};

fn main() {
    let now = time::Instant::now();
    let args = TopologyCheckerArgs::parse();
    match args.command {
        Commands::MustNotHaveDangles { lines, dangles } => {
            let vector_dataset = VectorDataset::new(lines.to_str().unwrap());
            let lines = vector_dataset.to_geo().unwrap();
            let lines = flatten_linestrings(lines);
            let result = lines.must_not_have_dangles();
            geometries_to_file(
                result,
                dangles.to_str().unwrap(),
                args.gdal_driver,
                Some(LayerOptions {
                    name: "dangles",
                    srs: vector_dataset.crs().as_ref(),
                    ..Default::default()
                }),
            );
        }
        Commands::MustNotIntersect {
            lines,
            single_points,
            collinear_lines,
        } => {
            let vector_dataset = VectorDataset::new(lines.to_str().unwrap());
            let lines = vector_dataset.to_geo().unwrap();
            let lines = flatten_linestrings(lines);
            let intersections = lines.must_no_intesect();
            geometries_to_file(
                intersections.0,
                collinear_lines.to_str().unwrap(),
                args.gdal_driver.clone(),
                Some(LayerOptions {
                    name: "collinear_lines",
                    srs: vector_dataset.crs().as_ref(),
                    ..Default::default()
                }),
            );
            geometries_to_file(
                intersections.1,
                single_points.to_str().unwrap(),
                args.gdal_driver.clone(),
                Some(LayerOptions {
                    name: "point_intersections",
                    srs: vector_dataset.crs().as_ref(),
                    ..Default::default()
                }),
            );
        }
        Commands::MustNotOverlap {
            geometry,
            overlaps,
            other,
        } => {
            let vector_dataset = VectorDataset::new(geometry.to_str().unwrap());
            let polygons = vector_dataset.to_geo().unwrap();
            let polygons = flatten_polygons(polygons);
            if let Some(other) = other {
                let other_polygons = VectorDataset::new(other.to_str().unwrap())
                    .to_geo()
                    .unwrap();
                let other_polygons = flatten_polygons(other_polygons);
                let result = polygons.must_not_overlap_with(other_polygons);
                geometries_to_file(
                    result,
                    overlaps.to_str().unwrap(),
                    args.gdal_driver,
                    Some(LayerOptions {
                        name: "overlaps",
                        srs: vector_dataset.crs().as_ref(),
                        ..Default::default()
                    }),
                )
            } else {
                let result = polygons.must_not_overlap();
                geometries_to_file(
                    result,
                    overlaps.to_str().unwrap(),
                    args.gdal_driver,
                    Some(LayerOptions {
                        name: "overlaps",
                        srs: vector_dataset.crs().as_ref(),
                        ..Default::default()
                    }),
                )
            }
        }
        Commands::GdalDrivers(command) => match command.command {
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
        Commands::Utilities(command) => match command.command {
            Utilities::ExplodeLinestrings { linestrings, lines } => {
                let dataset = VectorDataset::new(linestrings.to_str().unwrap());
                let geometry = dataset.to_geo().unwrap();
                let linestrings = flatten_linestrings(geometry);
                let exploded = flatten_lines(&linestrings);
                geometries_to_file(
                    exploded,
                    lines.to_str().unwrap(),
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
                let dataset = VectorDataset::new(linestrings.to_str().unwrap());
                let geometry = dataset.to_geo().unwrap();
                let linestrings = flatten_linestrings(geometry);
                let merged_linestrings = lines_to_linestring(linestrings);
                geometries_to_file(
                    merged_linestrings,
                    merged.to_str().unwrap(),
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
    println!("{:?}", now.elapsed());
}
