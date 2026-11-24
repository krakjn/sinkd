//! Shared pieces for the sinkd client and server.

mod auth;
mod control;
pub mod ignore;
pub mod manifest;
mod paths;
mod session;

pub use auth::{
    ensure_client_id, ensure_server_identity, issue_client, load_client_bundle, load_peers,
    save_client_bundle, save_peer, secrets_eq, ClientBundle, ServerIdentity,
};
pub use control::{notify, serve, socket_path};
pub use manifest::{conflict_name, dirty, dirty_with, preserve, scan, scan_with, Manifest};
pub use paths::{
    anchor_id, anchor_key, config_dir, expand, hostname, random_bytes, runtime_dir, sanitize,
    state_dir, user_name,
};
pub use ignore::Rules;
pub use session::{open_session, Role};

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Watch {
    pub path: PathBuf,
    pub share: bool,
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ServerSettings {
    pub sync_root: PathBuf,
    pub state_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub bind: String,
    pub zenoh_port: u16,
    pub data_port: u16,
    pub pair_port: u16,
    pub fixed_pair_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub server: String,
    pub state_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub zenoh_port: u16,
    pub pair_port: u16,
    pub watches: Vec<Watch>,
}

#[derive(Debug, Deserialize, Default)]
struct ServerFile {
    sync_root: Option<String>,
    bind: Option<String>,
    zenoh_port: Option<u16>,
    data_port: Option<u16>,
    pair_port: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AnchorSpec {
    Path(String),
    Table { path: String, #[serde(default)] ignore: Vec<String> },
}

#[derive(Debug, Deserialize, Default)]
struct ClientFile {
    server: Option<String>,
    zenoh_port: Option<u16>,
    pair_port: Option<u16>,
    anchors: Option<Vec<AnchorSpec>>,
    shares: Option<Vec<String>>,
}

#[derive(Debug, Default)]
pub struct ServerOverrides {
    pub sync_root: Option<PathBuf>,
    pub bind: Option<String>,
    pub zenoh_port: Option<u16>,
    pub data_port: Option<u16>,
    pub pair_port: Option<u16>,
    pub state_dir: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
    pub fixed_pair_code: Option<String>,
}

#[derive(Debug, Default)]
pub struct ClientOverrides {
    pub server: Option<String>,
    pub anchors: Vec<PathBuf>,
    pub shares: Vec<PathBuf>,
    pub zenoh_port: Option<u16>,
    pub pair_port: Option<u16>,
    pub state_dir: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
}

fn read_toml<T: for<'de> Deserialize<'de> + Default>(path: &std::path::Path) -> Result<T, String> {
    if !path.exists() {
        return Ok(T::default());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load_server(config: Option<&std::path::Path>, over: ServerOverrides) -> Result<ServerSettings, String> {
    let path = config.map_or_else(|| config_dir().join("server.toml"), PathBuf::from);
    let file = read_toml::<ServerFile>(&path)?;
    let sync_root = over
        .sync_root
        .or(file.sync_root.map(|s| expand(&s)))
        .unwrap_or_else(|| expand("~/sinkd-data"));
    Ok(ServerSettings {
        sync_root,
        state_dir: over.state_dir.unwrap_or_else(|| state_dir("sinkd-srv")),
        runtime_dir: over.runtime_dir.unwrap_or_else(runtime_dir),
        bind: over.bind.or(file.bind).unwrap_or_else(|| "0.0.0.0".to_string()),
        zenoh_port: over.zenoh_port.or(file.zenoh_port).unwrap_or(7447),
        data_port: over.data_port.or(file.data_port).unwrap_or(7448),
        pair_port: over.pair_port.or(file.pair_port).unwrap_or(7449),
        fixed_pair_code: over.fixed_pair_code,
    })
}

pub fn load_client(config: Option<&std::path::Path>, over: ClientOverrides) -> Result<ClientSettings, String> {
    let path = config.map_or_else(|| config_dir().join("client.toml"), PathBuf::from);
    let file = read_toml::<ClientFile>(&path)?;
    let state = over.state_dir.clone().unwrap_or_else(|| state_dir("sinkd"));
    let server = over
        .server
        .or(file.server)
        .or_else(|| remembered_host(&state))
        .ok_or_else(|| {
            "no server set; pass --server, set it in client.toml, or run `sinkd pair <host>`".to_string()
        })?;
    let mut watches = Vec::new();
    if over.anchors.is_empty() && over.shares.is_empty() {
        for spec in file.anchors.unwrap_or_default() {
            let (path, ignore) = match spec {
                AnchorSpec::Path(path) => (path, Vec::new()),
                AnchorSpec::Table { path, ignore } => (path, ignore),
            };
            watches.push(Watch { path: expand(&path), share: false, ignore });
        }
        for path in file.shares.unwrap_or_default() {
            watches.push(Watch { path: expand(&path), share: true, ignore: Vec::new() });
        }
    } else {
        for path in over.anchors {
            watches.push(Watch { path, share: false, ignore: Vec::new() });
        }
        for path in over.shares {
            watches.push(Watch { path, share: true, ignore: Vec::new() });
        }
    }
    if watches.is_empty() {
        return Err("no anchors; pass --anchor or set anchors in client.toml".into());
    }
    Ok(ClientSettings {
        server,
        state_dir: state,
        runtime_dir: over.runtime_dir.unwrap_or_else(runtime_dir),
        zenoh_port: over.zenoh_port.or(file.zenoh_port).unwrap_or(7447),
        pair_port: over.pair_port.or(file.pair_port).unwrap_or(7449),
        watches,
    })
}

pub fn remembered_host(state: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(state.join("server_host"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read_basis(state: &std::path::Path, anchor_key: &str) -> Option<u64> {
    let text = std::fs::read_to_string(state.join("basis").join(anchor_key)).ok()?;
    text.trim().parse().ok()
}

pub fn write_basis(state: &std::path::Path, anchor_key: &str, generation: u64) -> Result<(), String> {
    atomic_write(
        &state.join("basis").join(anchor_key),
        format!("{generation}\n").as_bytes(),
    )
}

pub fn manifest_path(state: &std::path::Path, anchor_key: &str) -> PathBuf {
    state.join("manifest").join(anchor_key)
}

#[cfg(test)]
mod anchor_parse {
    use super::load_client;

    #[test]
    fn table_keeps_ignore_and_a_string_list_still_works() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("client.toml");
        std::fs::write(
            &cfg,
            "server = \"hub\"\n[[anchors]]\npath = \"/data\"\nignore = [\"skip/\"]\n",
        )
        .unwrap();
        let settings = load_client(Some(&cfg), Default::default()).unwrap();
        assert_eq!(settings.watches[0].ignore, ["skip/"]);

        std::fs::write(&cfg, "server = \"hub\"\nanchors = [\"/data\"]\n").unwrap();
        let settings = load_client(Some(&cfg), Default::default()).unwrap();
        assert!(settings.watches[0].ignore.is_empty());
    }
}
