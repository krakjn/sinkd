//! Server-only runtime parameters and argv parsing.

use clap::ArgMatches;
use std::{fmt, path::PathBuf, sync::Arc};

use sinkd_core::{
    outcome::Outcome,
    parameters::{create_log_dir, get_log_path, DaemonType, SharedDaemonParams},
};

#[derive(Clone, Debug)]
pub struct ServerParameters {
    pub shared: SharedDaemonParams,
    pub server_config: Arc<PathBuf>,
}

impl fmt::Display for ServerParameters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.shared)?;
        writeln!(f, "server_config:{}", self.server_config.display())
    }
}

impl ServerParameters {
    pub fn from_matches(matches: &ArgMatches) -> Outcome<Self> {
        let debug = matches.get_count("debug");
        create_log_dir(debug)?;

        let (daemon_type, server_config) = match matches.subcommand() {
            Some(("init", init_m)) => {
                let windows = init_m.get_flag("windows-daemon");
                (
                    if windows {
                        DaemonType::WindowsServer
                    } else {
                        DaemonType::UnixServer
                    },
                    init_m.get_one::<String>("server-config"),
                )
            }
            Some(_) => {
                let windows = matches.get_flag("windows-daemon");
                (
                    if windows {
                        DaemonType::WindowsServer
                    } else {
                        DaemonType::UnixServer
                    },
                    matches.get_one::<String>("server-config"),
                )
            }
            None => return sinkd_core::bad!("expected subcommand"),
        };

        let debug_level = match debug {
            1 | 2 => debug,
            d if d > 2 => {
                println!("debug only has two levels");
                2
            }
            _ => 0,
        };

        let verbosity = match (debug, matches.get_count("verbose")) {
            (d, _) if d > 0 => 4,
            (_, 0) => 2,
            (_, v) => v,
        };

        let shared = SharedDaemonParams {
            daemon_type,
            verbosity,
            debug: debug_level,
            log_path: get_log_path(debug, daemon_type),
        };

        let params = Self {
            shared,
            server_config: Arc::new(
                server_config
                    .map(|s| PathBuf::from(s.as_str()))
                    .unwrap_or_default(),
            ),
        };

        if params.shared.debug > 0 {
            println!("{params}");
        }

        Ok(params)
    }
}
