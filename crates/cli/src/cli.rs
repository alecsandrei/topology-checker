use clap::{Arg, ArgAction, Command, value_parser};
use std::path::PathBuf;
use topology_checker_core::Rule;

fn input() -> Arg {
    Arg::new("input")
        .required(true)
        .value_parser(value_parser!(PathBuf))
}
fn output(required: bool) -> Arg {
    Arg::new("output")
        .short('o')
        .long("output")
        .required(required)
        .value_parser(value_parser!(PathBuf))
}
fn layer() -> Arg {
    Arg::new("layer")
        .long("layer")
        .help("Select one physical dataset layer; otherwise use every layer separately")
}
pub fn command() -> Command {
    let mut command = Command::new("topology-checker")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Validate feature-aware vector topology")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .arg(
            Arg::new("json")
                .long("json")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Print a machine-readable report"),
        )
        .arg(
            Arg::new("summary")
                .long("summary")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("With --json, leave out the list of violations; the table is as ever"),
        )
        .arg(
            Arg::new("overwrite")
                .long("overwrite")
                .global(true)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("driver")
                .long("driver")
                .global(true)
                .help("Explicit vector output format or optional GDAL driver"),
        )
        .arg(
            Arg::new("tolerance")
                .long("tolerance")
                .global(true)
                .allow_hyphen_values(true)
                .value_parser(value_parser!(f64))
                .help("Distance tolerance in input coordinate units"),
        );
    for kind in ["point", "line", "polygon", "geometry"] {
        let mut group = Command::new(kind).subcommand_required(true);
        for rule in Rule::ALL
            .iter()
            .copied()
            .filter(|r| r.name().starts_with(kind))
        {
            let name = rule
                .name()
                .split_once('/')
                .expect("registry rule names include a group")
                .1;
            let mut sub = Command::new(name)
                .about(rule.description())
                .arg(input())
                .arg(output(false))
                .arg(layer());
            if rule.inputs().1.is_some() {
                sub = sub
                    .arg(
                        Arg::new("other")
                            .long("other")
                            .required(true)
                            .value_parser(value_parser!(PathBuf)),
                    )
                    .arg(Arg::new("other-layer").long("other-layer"));
            }
            group = group.subcommand(sub);
        }
        command = command.subcommand(group);
    }
    command
        .subcommand(
            Command::new("validate")
                .about("Validate a TOML topology definition")
                .arg(input())
                .arg(output(false)),
        )
        .subcommand(Command::new("rules").about("List rule definitions and GIS equivalents"))
        .subcommand(
            Command::new("drivers")
                .about("List vector formats and optional GDAL drivers")
                .arg(Arg::new("read").long("read").action(ArgAction::SetTrue))
                .arg(Arg::new("write").long("write").action(ArgAction::SetTrue)),
        )
        .subcommand(
            Command::new("utilities")
                .subcommand_required(true)
                .subcommand(
                    Command::new("merge-lines")
                        .arg(input())
                        .arg(layer())
                        .arg(output(true)),
                )
                .subcommand(
                    Command::new("explode-lines")
                        .arg(input())
                        .arg(layer())
                        .arg(output(true)),
                ),
        )
}
