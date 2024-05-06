mod args {
    use clap::{Args, Parser, Subcommand};
    use std::path::PathBuf;

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
            /// Input lines
            lines: PathBuf,
            /// Output dangles
            dangles: PathBuf,
        },
        #[command(arg_required_else_help(true))]
        MustNotIntersect {
            /// Input lines
            lines: PathBuf,
            /// Output point intersections
            single_points: PathBuf,
            /// Output line intersections
            collinear_lines: PathBuf,
        },
        #[command(arg_required_else_help(true))]
        MustNotOverlap {
            /// The input geometry
            geometry: PathBuf,
            /// The output overlaps
            overlaps: PathBuf,
            /// Optional geometry to check against. By default compares to itself
            other: Option<PathBuf>,
        },
        /// Print the allowed GDAL drivers
        GdalDrivers(GdalDriversCommand),
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
}

use args::{Commands, Drivers, TopologyCheckerArgs};
use clap::Parser;
use gdal::LayerOptions;
use topology_checker::rules::{MustNotHaveDangles, MustNotIntersect, MustNotOverlap};
use topology_checker::utils::{
    flatten_linestrings, flatten_polygons, geometries_to_file, GdalDrivers,
};
use topology_checker::VectorDataset;

fn main() {
    let args = TopologyCheckerArgs::parse();
    match args.command {
        Commands::MustNotHaveDangles { lines, dangles } => {
            let vector_dataset = VectorDataset::new(lines.to_str().unwrap());
            let lines = vector_dataset.to_geo().unwrap();
            let lines = flatten_linestrings(lines);
            let result = lines.there_are_no_dangles();
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
    }
}
