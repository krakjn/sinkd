//! `sinkd-srv init` — scaffold server config from the template.

const SERVER_TEMPLATE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/conf.tmpl"));

use clap::ArgMatches;
use sinkd_core::{
    init::{InitOptions, SERVER_TEMPLATE_DISK, render},
    outcome::Outcome,
};

use crate::{config, parameters::ServerParameters};

pub fn run(sub: &ArgMatches, parameters: &ServerParameters) -> Outcome<()> {
    let force = sub.get_flag("force");
    let location = sub.get_one::<String>("location").cloned().unwrap_or_else(|| {
        config::default_sync_location(parameters.shared.debug)
            .to_string_lossy()
            .into_owned()
    });
    let target = config::get_server_config_path(parameters)?;

    render(&InitOptions {
        target_path: target,
        template_disk: Some(std::path::Path::new(SERVER_TEMPLATE_DISK)),
        template_embedded: SERVER_TEMPLATE,
        substitutions: &[("location", location)],
        force,
    })
}
