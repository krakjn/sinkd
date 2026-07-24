use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::parameters::ServerParameters;
use sinkd_core::{bad, config::RsyncOptions, ensure_parent_dir, outcome::Outcome};

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    pub location: PathBuf,
    #[serde(default, alias = "rsync")]
    pub rsync_options: RsyncOptions,
}

/// Default sync root when no server config file exists (debug / fallback).
#[must_use]
pub fn default_sync_location(debug: u8) -> PathBuf {
    if debug > 0 {
        PathBuf::from("/tmp/sinkd/srv")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(r"C:\Program Files\sinkd\srv")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/opt/sinkd/srv")
    } else {
        PathBuf::from("/srv/sinkd")
    }
}

pub fn get_server_config_path(params: &ServerParameters) -> Outcome<PathBuf> {
    let cfg_path: PathBuf = if params.shared.debug > 0 {
        PathBuf::from("/tmp/sinkd/server.conf")
    } else if !params.server_config.as_os_str().is_empty() {
        params.server_config.as_ref().clone()
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/opt/sinkd/server.conf")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(r"C:\ProgramData\sinkd\server.conf")
    } else {
        PathBuf::from("/etc/sinkd.server.conf")
    };
    ensure_parent_dir(&cfg_path)?;
    Ok(cfg_path)
}

pub fn load_server_config(path: &Path) -> Outcome<ServerConfig> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("cannot read server config {}: {e}", path.display()))?;
    match toml::from_str(&raw) {
        Err(error) => bad!("cannot parse server config {}: {}", path.display(), error),
        Ok(cfg) => Ok(cfg),
    }
}

pub fn load(params: &ServerParameters) -> Outcome<ServerConfig> {
    let path = get_server_config_path(params)?;
    load_server_config(&path)
}
