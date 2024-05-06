mod args {
    use clap::{Parser, Subcommand};
    use std::path::PathBuf;

    #[derive(Debug, Parser)]
    #[clap(author, version, about)]
    pub struct TopologyCheckerArgs {
        #[clap(subcommand)]
        pub rule: Rule,
    }

    #[derive(Debug, Subcommand)]
    pub enum Rule {
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
    }
}

use args::{Rule, TopologyCheckerArgs};
use clap::Parser;
use topology_checker::geometries_to_file;
use topology_checker::rules::{MustNotHaveDangles, MustNotIntersect, MustNotOverlap};
use topology_checker::utils::{flatten_linestrings, flatten_polygons};
use topology_checker::VectorDataset;

fn main() {
    let args = TopologyCheckerArgs::parse();
    match args.rule {
        Rule::MustNotHaveDangles { lines, dangles } => {
            let lines = VectorDataset::new(lines.to_str().unwrap())
                .to_geo()
                .unwrap();
            let lines = flatten_linestrings(lines);
            let result = lines.there_are_no_dangles();
            geometries_to_file(result, dangles.to_str().unwrap());
        }
        Rule::MustNotIntersect {
            lines,
            single_points,
            collinear_lines,
        } => {
            let lines = VectorDataset::new(lines.to_str().unwrap())
                .to_geo()
                .unwrap();
            let lines = flatten_linestrings(lines);
            let intersections = lines.must_no_intesect();
            geometries_to_file(intersections.0, collinear_lines.to_str().unwrap());
            geometries_to_file(intersections.1, single_points.to_str().unwrap());
        }
        Rule::MustNotOverlap {
            geometry,
            overlaps,
            other,
        } => {
            let polygons = VectorDataset::new(geometry.to_str().unwrap())
                .to_geo()
                .unwrap();
            let polygons = flatten_polygons(polygons);
            if let Some(other) = other {
                let other_polygons = VectorDataset::new(other.to_str().unwrap())
                    .to_geo()
                    .unwrap();
                let other_polygons = flatten_polygons(other_polygons);
                let result = polygons.must_not_overlap_with(other_polygons);
                geometries_to_file(result, overlaps.to_str().unwrap())
            } else {
                let result = polygons.must_not_overlap();
                geometries_to_file(result, overlaps.to_str().unwrap())
            }
        }
    }
}
