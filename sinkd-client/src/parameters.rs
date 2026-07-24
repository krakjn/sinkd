//! Client-only runtime parameters and argv parsing.

use clap::{ArgMatches};
use std::{fmt, path::PathBuf, sync::Arc};

use sinkd_core::{
    outcome::Outcome,
    parameters::{create_log_dir, get_log_path, DaemonType, SharedDaemonParams},
};

#[derive(Clone, Debug)]
pub struct ClientParameters {
    pub shared: SharedDaemonParams,
    pub system_config: Arc<PathBuf>,
    pub user_configs: Arc<Vec<PathBuf>>,
    pub client_state_dir_override: Option<PathBuf>,
}

impl fmt::Display for ClientParameters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.shared)?;
        write!(
            f,
            "system_config:{}\nuser configs: [{}]\nclient_state_dir_override:{}\n",
            self.system_config.display(),
            self.user_configs
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            self.client_state_dir_override
                .as_ref()
                .map(|p| p.display().to_string())
                .as_deref()
                .unwrap_or("(default)")
        )
    }
}

impl ClientParameters {
    pub fn from_matches(matches: &ArgMatches) -> Outcome<Self> {
        let debug = matches.get_count("debug");
        create_log_dir(debug)?;

        let (daemon_type, system_config, user_configs, client_state_dir_override) =
            match matches.subcommand() {
                Some(("init", init_m)) => {
                    let client_state_dir_override = init_m
                        .get_one::<String>("client-state-dir")
                        .map(|s| PathBuf::from(s.trim()))
                        .filter(|p| !p.as_os_str().is_empty());
                    (
                        DaemonType::UnixClient,
                        init_m.get_one::<String>("system-config"),
                        init_m.get_many::<String>("user-configs"),
                        client_state_dir_override,
                    )
                }
                Some(_) => {
                    let windows = matches.get_flag("windows-daemon");
                    let daemon_type = if windows {
                        DaemonType::WindowsClient
                    } else {
                        DaemonType::UnixClient
                    };
                    let client_state_dir_override = matches
                        .get_one::<String>("client-state-dir")
                        .map(|s| PathBuf::from(s.trim()))
                        .filter(|p| !p.as_os_str().is_empty());
                    (
                        daemon_type,
                        matches.get_one::<String>("system-config"),
                        matches.get_many::<String>("user-configs"),
                        client_state_dir_override,
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
            system_config: Arc::new(
                system_config
                    .map(|s| PathBuf::from(s.as_str()))
                    .unwrap_or_default(),
            ),
            user_configs: Arc::new(
                user_configs
                    .map(|configs| configs.map(|s| PathBuf::from(s.as_str())).collect())
                    .unwrap_or_default(),
            ),
            client_state_dir_override,
        };

        if params.shared.debug > 0 {
            println!("{params}");
        }

        Ok(params)
    }
}
