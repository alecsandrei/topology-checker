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
    }

    #[derive(Debug, Args)]
    pub struct PointCommand {
        #[clap(subcommand)]
        pub command: PointRules,
    }

    #[derive(Debug, Args)]
    pub struct LineCommand {
        #[clap(subcommand)]
        pub command: LineRules,
    }

    #[derive(Debug, Args)]
    pub struct PolygonCommand {
        #[clap(subcommand)]
        pub command: PolygonRules,
    }

    #[derive(Debug, Args)]
    pub struct GeometryCommand {
        #[clap(subcommand)]
        pub command: GeometryRules,
    }

    #[derive(Debug, Args)]
    pub struct GdalDriversCommand {
        #[clap(subcommand)]
        pub command: Drivers,
    }

    #[derive(Debug, Subcommand)]
    pub enum PointRules {
        #[command(arg_required_else_help(true))]
        MustNotOverlap {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            points: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Optional geometry to check against. By default compares
            /// against other features in the input.
            other: Option<PathBuf>,
        },
    }

    #[derive(Debug, Subcommand)]
    pub enum LineRules {
        #[command(arg_required_else_help(true))]
        MustNotOverlap {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input lines
            lines: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: PathBuf,
            /// Whether or not to check for self overlaps.
            /// This can't be true if 'other' has been specified.
            #[arg(long, short)]
            self_overlap: bool,
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
    }

    #[derive(Debug, Subcommand)]
    pub enum PolygonRules {
        #[command(arg_required_else_help(true))]
        MustNotOverlap {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input polygons
            polygons: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output overlaps
            overlaps: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Optional geometry to check against. By default compares to itself
            other: Option<PathBuf>,
        },
    }

    #[derive(Debug, Subcommand)]
    pub enum GeometryRules {
        #[command(arg_required_else_help(true))]
        MustNotBeMultipart {
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// Input geometries
            geometries: PathBuf,
            #[arg(value_parser = parse_key_val::<String, PathBuf>)]
            /// The output multipart geometries
            multiparts: PathBuf,
        },
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

use args::{
    Commands, Drivers, GeometryRules, LineRules, PointRules, PolygonRules, TopologyCheckerArgs,
    Utilities,
};
use clap::Parser;
use gdal::{vector::ToGdal, LayerOptions};
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
    GeometryError, VectorDataset,
};

fn main() {
    let args = TopologyCheckerArgs::parse();
    match args.command {
        Commands::Point(command) => match command.command {
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
                    if result.is_valid() {
                        println!("No errors found.")
                    } else {
                        result.unwrap_err_point().to_file(
                            &overlaps,
                            args.gdal_driver,
                            Some(LayerOptions {
                                name: "overlaps",
                                srs: vector_dataset.crs().as_ref(),
                                ..Default::default()
                            }),
                        )
                    }
                } else {
                    let result = points.must_not_overlap();
                    if result.is_valid() {
                        println!("No errors found.")
                    } else {
                        result.unwrap_err_point().to_file(
                            &overlaps,
                            args.gdal_driver,
                            Some(LayerOptions {
                                name: "overlaps",
                                srs: vector_dataset.crs().as_ref(),
                                ..Default::default()
                            }),
                        )
                    }
                }
            }
        },
        Commands::Line(command) => match command.command {
            LineRules::MustNotHaveDangles { lines, dangles } => {
                let vector_dataset = VectorDataset::new(&lines);
                let lines = vector_dataset.to_geo().unwrap();
                let lines = flatten_linestrings(lines);
                let result = lines.must_not_have_dangles();
                if result.is_valid() {
                    println!("No errors found.")
                } else {
                    result.unwrap_err_point().to_file(
                        &dangles,
                        args.gdal_driver,
                        Some(LayerOptions {
                            name: "dangles",
                            srs: vector_dataset.crs().as_ref(),
                            ..Default::default()
                        }),
                    );
                }
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
                if result.is_valid() {
                    println!("No errors found.")
                } else {
                    let errors = result.unwrap_err();
                    for error in errors.into_iter() {
                        if let GeometryError::Point(_) = error {
                            error.to_file(
                                &single_points,
                                args.gdal_driver.clone(),
                                Some(LayerOptions {
                                    name: "intersections",
                                    srs: vector_dataset.crs().as_ref(),
                                    ..Default::default()
                                }),
                            );
                        } else if let GeometryError::LineString(_) = error {
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
                    if result.is_valid() {
                        println!("No errors found.")
                    } else {
                        result.unwrap_err_linestring().to_file(
                            &overlaps,
                            args.gdal_driver.clone(),
                            Some(LayerOptions {
                                name: "overlaps",
                                srs: vector_dataset.crs().as_ref(),
                                ..Default::default()
                            }),
                        );
                    }
                } else {
                    let result;
                    if self_overlap {
                        result = lines.must_not_self_overlap();
                    } else {
                        result = lines.must_not_overlap();
                    }
                    if result.is_valid() {
                        println!("No errors found.")
                    } else {
                        result.unwrap_err_linestring().to_file(
                            &overlaps,
                            args.gdal_driver.clone(),
                            Some(LayerOptions {
                                name: "overlaps",
                                srs: vector_dataset.crs().as_ref(),
                                ..Default::default()
                            }),
                        );
                    }
                }
            }
        },
        Commands::Polygon(command) => match command.command {
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
                    if result.is_valid() {
                        println!("No errors found.")
                    } else {
                        result.unwrap_err_polygon().to_file(
                            &overlaps,
                            args.gdal_driver,
                            Some(LayerOptions {
                                name: "overlaps",
                                srs: vector_dataset.crs().as_ref(),
                                ..Default::default()
                            }),
                        )
                    }
                } else {
                    let result = polygons.must_not_overlap();
                    if result.is_valid() {
                        println!("No errors found.")
                    } else {
                        result.unwrap_err_polygon().to_file(
                            &overlaps,
                            args.gdal_driver,
                            Some(LayerOptions {
                                name: "overlaps",
                                srs: vector_dataset.crs().as_ref(),
                                ..Default::default()
                            }),
                        )
                    }
                }
            }
        },
        Commands::Geometry(command) => match command.command {
            GeometryRules::MustNotBeMultipart {
                geometries,
                multiparts,
            } => {
                let dataset = VectorDataset::new(&geometries);
                let geometry = dataset.to_geo().unwrap();
                let result = geometry.must_not_be_multipart();
                if result.is_valid() {
                    println!("No errors found.")
                } else {
                    result.unwrap_err().into_iter().next().unwrap().to_file(
                        &multiparts,
                        args.gdal_driver,
                        Some(LayerOptions {
                            name: "multiparts",
                            srs: dataset.crs().as_ref(),
                            ..Default::default()
                        }),
                    )
                }
            }
        },
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
