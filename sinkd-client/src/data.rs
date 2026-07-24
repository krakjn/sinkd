use std::{collections::HashMap, path::PathBuf, time::{Duration, Instant}};
use serde::{Serialize, Deserialize};
use sinkd_core::config::RsyncOptions;

#[derive(Debug, Clone)]
pub struct Inode {
    pub interval: Duration,
    pub last_event: Instant,
    pub rsync_options: RsyncOptions,
}

pub type InodeMap = HashMap<PathBuf, Inode>;

#[derive(Debug, Serialize, Deserialize)]
pub struct Share {
    pub path: PathBuf,
    pub users: Vec<String>,
    pub interval: Option<u64>,
    pub excludes: Option<Vec<String>>,
    pub rsync_options: Option<RsyncOptions>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Anchor {
    pub path: PathBuf,
    pub interval: Option<u64>,
    pub excludes: Option<Vec<String>>,
    pub rsync_options: Option<RsyncOptions>,
}

impl Anchor {
    pub fn from(path: PathBuf) -> Self {
        Self {
            path,
            interval: Some(5),
            excludes: None,
            rsync_options: None,
        }
    }
}