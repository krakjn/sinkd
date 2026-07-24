//! **Two configuration surfaces (by design):**
//! - **Client** — system TOML (`/etc/sinkd.conf` by default, or `--sys-cfg`) plus per-user TOML files;
//!   parsed in the `sinkd` client crate. `server_addr` in the system file is the sync target for clients.
//! - **Server** — runtime sync root under `/srv/sinkd` (or debug path) and `generation_state.toml` there; the
//!   server does **not** load client TOML anchor lists for queue/dedup logic.

use serde::{Deserialize, Serialize};

#[derive(PartialEq)]
pub enum ParseError {
    FileNotFound,
    InvalidSyntax(String),
    NotSupportedOption(String),
    // NoUserFound,
}

// macro_rules! reject_unsupported_rsync_fields {
//     ($cfg:expr, $($field:ident),+ $(,)?) => {
//         $(
//             if $cfg.$field.is_some() {
//                 return Err(format!("unsupported rsync flag `{}` in config", stringify!($field)));
//             }
//         )+
//     };
// }


#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RsyncOptions {
    pub checksum: Option<bool>,
    pub compress: Option<bool>,
    pub bwlimit: Option<String>,
    pub partial: Option<bool>,
    pub delete_excluded: Option<bool>,
    pub max_size: Option<String>,
    pub min_size: Option<String>,
    pub ignore_existing: Option<bool>,
    pub size_only: Option<bool>,
    pub stats: Option<bool>,
    pub owner: Option<toml::Value>,
    pub group: Option<toml::Value>,
    pub devices: Option<toml::Value>,
    pub specials: Option<toml::Value>,
    pub super_user: Option<toml::Value>,
    pub fake_super: Option<toml::Value>,
    pub rsh: Option<toml::Value>,
    pub address: Option<toml::Value>,
    pub port: Option<toml::Value>,
    pub sockopts: Option<toml::Value>,
    pub remove_source_files: Option<toml::Value>,
    pub files_from: Option<toml::Value>,
    pub include_from: Option<toml::Value>,
    pub exclude_from: Option<toml::Value>,
    pub from0: Option<toml::Value>,
    pub usermap: Option<toml::Value>,
    pub groupmap: Option<toml::Value>,
    pub chown: Option<toml::Value>,
    pub chmod: Option<toml::Value>,
}
