use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::data::{Anchor, Inode, InodeMap, Share};
use crate::parameters::ClientParameters;
use sinkd_core::{bad, config::RsyncOptions, ensure_parent_dir, ipc::STATUS_DEST, outcome::Outcome};

#[cfg(unix)]
use users::os::unix::UserExt;

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemConfig {
    pub server_addr: String,
    pub users: Vec<String>, // to pick up user configs
    // the rest is multi-user paths to allow file sync
    pub shares: Option<Vec<Share>>,
    pub anchors: Option<Vec<Anchor>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserConfig {
    pub anchors: Vec<Anchor>,
}

struct RuntimeConfig {
    system: SystemConfig,
    users: Vec<UserConfig>,
}

#[cfg(unix)]
fn user_home_path(user: &users::User) -> PathBuf {
    user.home_dir().to_path_buf()
}

/// Current login name as `String` (via the `users` crate).
pub fn current_username() -> Outcome<String> {
    let os_name = users::get_current_username().ok_or("no current username")?;
    Ok(os_name.to_string_lossy().into_owned())
}

// pass None for current user
fn get_user_home(user: Option<&str>) -> Outcome<PathBuf> {
    if let Some(name) = user {
        let user = users::get_user_by_name(name).ok_or_else(|| format!("unknown user: {name}"))?;
        Ok(user_home_path(&user))
    } else {
        let name = current_username()?;
        let user = users::get_user_by_name(&name)
            .ok_or_else(|| format!("unknown user: {name}"))?;
        Ok(user_home_path(&user))
    }
}

pub fn get_system_config_path(params: &ClientParameters) -> Outcome<PathBuf> {
    let cfg_path: PathBuf;
    if params.shared.debug > 0 {
        cfg_path = get_user_home(None)?.join("__sinkd").join("system.conf");
    } else if !params.system_config.as_os_str().is_empty() {
        cfg_path = params.system_config.as_ref().clone();
    } else {
        // unfortunately, each OS has its own default path for the system config
        cfg_path = if cfg!(target_os = "macos") {
            PathBuf::from("/opt/sinkd/sinkd.conf")
        } else if cfg!(target_os = "windows") {
            PathBuf::from(r"C:\ProgramData\sinkd\sinkd.conf")
        } else {
            PathBuf::from("/etc/sinkd.conf")
        };
    }
    ensure_parent_dir(&cfg_path)?;
    Ok(cfg_path)
}

/// User config path for a named account (`~/.config/sinkd/sinkd.conf` in production).
pub fn user_config_path_for_username(username: &str, debug: u8) -> Outcome<PathBuf> {
    get_user_config_path_from_username(username, debug)
}

fn get_user_config_path_from_username(username: &str, debug: u8) -> Outcome<PathBuf> {
    let home = get_user_home(Some(username))?;
    let cfg_path = if debug > 0 {
        home.join("__sinkd").join("user.conf")
    } else {
        home.join(".config").join("sinkd").join("sinkd.conf")
    };
    ensure_parent_dir(&cfg_path)?;
    Ok(cfg_path)
}

pub fn get_user_config_paths(params: &ClientParameters) -> Outcome<Vec<PathBuf>> {
    let mut cfg_paths: Vec<PathBuf> = Vec::new();
    if params.shared.debug > 0 {
        let home = get_user_home(None)?;
        let cfg_path = home.join("__sinkd").join("user.conf");
        ensure_parent_dir(&cfg_path)?;
        cfg_paths.push(cfg_path);
    } else if !params.user_configs.is_empty() {
        for path in params.user_configs.iter() {
            ensure_parent_dir(path)?;
            cfg_paths.push(path.clone());
        }
    } else {
        let home = get_user_home(None)?;
        // enforce user convention regardless of OS! ~/.config is awesome!
        let cfg_path = home.join(".config").join("sinkd").join("sinkd.conf");
        ensure_parent_dir(&cfg_path)?;
        cfg_paths.push(cfg_path);
    }
    Ok(cfg_paths)
}

pub fn load_system_config(path: &Path) -> Outcome<SystemConfig> {
    match fs::read_to_string(path) {
        Err(_) => bad!("file not found: {}", path.display()),
        Ok(output) => match toml::from_str(&output) {
            Err(error) => bad!("cannot parse system config {}: {}", path.display(), error),
            Ok(toml_parsed) => Ok(toml_parsed),
        },
    }
}

pub fn load_user_config(path: &Path) -> Outcome<UserConfig> {
    match fs::read_to_string(path) {
        Err(_) => bad!("file not found: {}", path.display()),
        Ok(output) => match toml::from_str(&output) {
            Err(error) => bad!("cannot parse user config {}: {}", path.display(), error),
            Ok(toml_parsed) => Ok(toml_parsed),
        },
    }
}

pub fn load_inode_map(params: &ClientParameters) -> Outcome<InodeMap> {
    get_inode_map(&load(params)?)
}

/// `server_addr` from the system config (rsync target host or local mount path).
pub fn server_addr(params: &ClientParameters) -> Outcome<String> {
    let path = get_system_config_path(params)?;
    Ok(load_system_config(&path)?.server_addr)
}

/// Sentinel `dest_path` on server status broadcasts (not a filesystem path).
pub fn is_status_dest(path: &Path) -> bool {
    path.as_os_str() == STATUS_DEST
}

fn load(params: &ClientParameters) -> Outcome<RuntimeConfig> {
    let debug = params.shared.debug;
    let system_config_path = get_system_config_path(params)?;
    let system_config = load_system_config(&system_config_path)?;
    let mut users: Vec<UserConfig> = Vec::new();

    // passed args take precedence
    if params.user_configs.is_empty() {
        for user in &system_config.users {
            let user_config_path = get_user_config_path_from_username(user, debug)?;
            let user_config = load_user_config(&user_config_path)?;
            users.push(user_config);
        }
    } else {
        let user_config_paths = get_user_config_paths(params)?;
        for user_config_path in user_config_paths {
            let user_config = load_user_config(&user_config_path)?;
            users.push(user_config);
        }
    }

    Ok(RuntimeConfig {
        system: system_config,
        users,
    })
}

fn get_inode_map(runtime_config: &RuntimeConfig) -> Outcome<InodeMap> {
    let mut inode_map: InodeMap = HashMap::new();

    for user in &runtime_config.users {
        for anchor in &user.anchors {
            inode_map.entry(anchor.path.clone()).or_insert(Inode {
                interval: Duration::from_secs(anchor.interval.unwrap_or(5)),
                last_event: Instant::now(),
                rsync_options: anchor.rsync_options.clone().unwrap_or(RsyncOptions::default()),
            });
        }
    }

    if let Some(anchors) = &runtime_config.system.anchors {
        for anchor in anchors {
            inode_map.entry(anchor.path.clone()).or_insert(Inode {
                interval: Duration::from_secs(anchor.interval.unwrap_or(5)),
                last_event: Instant::now(),
                rsync_options: anchor.rsync_options.clone().unwrap_or(RsyncOptions::default()),
            });
        }
    }
    Ok(inode_map)
}

pub fn save_system_config(path: &Path, cfg: &SystemConfig) -> Outcome<()> {
    let serialized =
        toml::to_string_pretty(cfg).map_err(|e| format!("cannot serialize system config: {e}"))?;
    fs::write(path, serialized)?;
    Ok(())
}

pub fn save_user_config(path: &Path, cfg: &UserConfig) -> Outcome<()> {
    let serialized =
        toml::to_string_pretty(cfg).map_err(|e| format!("cannot serialize user config: {e}"))?;
    fs::write(path, serialized)?;
    Ok(())
}
