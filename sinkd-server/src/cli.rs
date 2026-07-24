use clap::{Arg, ArgAction, ArgMatches, Command};
use std::process::ExitCode;

use sinkd_core::{fancy_error, outcome::Outcome};

use crate::{parameters::ServerParameters, server};

pub fn egress<T>(outcome: Outcome<T>) -> ExitCode {
    match outcome {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            fancy_error!("ERROR: {}", e);
            ExitCode::FAILURE
        }
    }
}

#[must_use]
pub fn build_command() -> Command {
    let verbose_arg = Arg::new("verbose")
        .short('v')
        .action(ArgAction::Count)
        .help("log verbosity: v=error, vv=warn (default), vvv=info, vvvv=debug")
        .global(true);

    let debug_arg = Arg::new("debug")
        .short('d')
        .long("debug")
        .action(ArgAction::Count)
        .help("debug logs under /tmp/sinkd; -dd also enables Zenoh/IPC crate logs in the file")
        .global(true);

    Command::new("sinkd-srv")
        .about("Deployable Cloud: server side")
        .version(env!("CARGO_PKG_VERSION"))
        .arg(verbose_arg)
        .arg(debug_arg)
        .arg(
            Arg::new("windows-daemon")
                .long("windows-daemon")
                .action(ArgAction::SetTrue)
                .hide(true)
                .global(true),
        )
        .arg(
            Arg::new("server-config")
                .help("server TOML (overrides default path)")
                .long("server-config")
                .short('c')
                .num_args(1)
                .global(true),
        )
        .subcommand_required(true)
        .subcommand(Command::new("start").about("Start the server daemon"))
        .subcommand(Command::new("restart").about("Restart the server daemon"))
        .subcommand(Command::new("stop").about("Stop the server daemon"))
        .subcommand(Command::new("ls").about("Show server sync root and generation state"))
        .subcommand(
            Command::new("init")
                .about("Scaffold the server config file from the embedded template")
                .arg(
                    Arg::new("location")
                        .long("location")
                        .value_name("PATH")
                        .num_args(1)
                        .help("Sync root directory to write into the config (default: OS-specific /srv/sinkd)"),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .action(ArgAction::SetTrue)
                        .help("Overwrite existing config file"),
                ),
        )
}

#[must_use]
pub fn dispatch(matches: &ArgMatches, parameters: &ServerParameters) -> ExitCode {
    match matches.subcommand() {
        Some(("start", _)) => egress(server::start(parameters)),
        Some(("restart", _)) => egress(server::restart(parameters)),
        Some(("stop", _)) => egress(server::stop()),
        Some(("ls", _)) => egress(server::ls(parameters)),
        Some(("init", s)) => egress(crate::init::run(s, parameters)),
        _ => {
            fancy_error!("unknown subcommand");
            ExitCode::FAILURE
        }
    }
}
