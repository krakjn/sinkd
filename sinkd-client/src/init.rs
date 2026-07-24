const SYSTEM_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/system.conf.tmpl"));
const USER_TEMPLATE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/conf.tmpl"));

use std::path::{Path, PathBuf};

use clap::ArgMatches;
use sinkd_core::{
    init::{
        InitOptions, SYSTEM_TEMPLATE_DISK, USER_TEMPLATE_DISK, render, toml_string_array_body,
    },
    outcome::Outcome,
    bad,
};

use crate::{config, parameters::ClientParameters};

pub fn run(sub: &ArgMatches, parameters: &ClientParameters) -> Outcome<()> {
    let server_addr = sub
        .get_one::<String>("server-addr")
        .map(String::as_str)
        .ok_or("--server-addr is required")?;
    let user = match sub.get_one::<String>("user") {
        Some(s) => s.clone(),
        None => config::current_username()?,
    };
    let watch_arg = sub
        .get_one::<String>("watch")
        .ok_or("--watch is required")?;

    let interval: u64 = match sub.get_one::<String>("interval") {
        Some(s) => match s.parse::<u64>() {
            Ok(interval) => interval,
            Err(e) => {
                return bad!(format!("invalid interval: {e}, defaulting to 5 seconds"));
            }
        }
        None => 5,
    };

    let force = sub.get_flag("force");

    let watch = PathBuf::from(watch_arg);
    let sys_target = config::get_system_config_path(parameters)?;
    let user_target = config::user_config_path_for_username(&user, parameters.shared.debug)?;
    let users = vec![user];
    let users_body = toml_string_array_body(&users);

    render(&InitOptions {
        target_path: sys_target,
        template_disk: Some(Path::new(SYSTEM_TEMPLATE_DISK)),
        template_embedded: SYSTEM_TEMPLATE,
        substitutions: &[
            ("server_addr", server_addr.to_string()),
            ("users", users_body),
        ],
        force,
    })?;

    render(&InitOptions {
        target_path: user_target,
        template_disk: Some(Path::new(USER_TEMPLATE_DISK)),
        template_embedded: USER_TEMPLATE,
        substitutions: &[
            ("watch", watch.display().to_string()),
            ("interval", interval.to_string()),
        ],
        force,
    })
}
