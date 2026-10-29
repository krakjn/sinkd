use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub files: BTreeMap<String, FileMeta>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMeta {
    pub size: u64,
    pub mtime: i64,
}

pub fn scan(root: &Path) -> Manifest {
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    Manifest { files }
}

fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, FileMeta>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.contains(".sinkd-conflict-") {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            walk(root, &path, files);
            continue;
        }
        if !kind.is_file() && !kind.is_symlink() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(rel) = path.strip_prefix(root) else { continue };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
        files.insert(
            rel.to_string_lossy().replace('\\', "/"),
            FileMeta { size: meta.len(), mtime },
        );
    }
}

#[must_use]
pub fn dirty(root: &Path, manifest: &Manifest) -> Vec<String> {
    let now = scan(root);
    let mut out = Vec::new();
    for (path, meta) in &now.files {
        if manifest.files.get(path) != Some(meta) {
            out.push(path.clone());
        }
    }
    for path in manifest.files.keys() {
        if !now.files.contains_key(path) {
            out.push(path.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

#[must_use]
pub fn conflict_name(file_name: &str, host: &str, unix_secs: u64) -> String {
    format!("{file_name}.sinkd-conflict-{host}-{unix_secs}")
}

/// Copy `rels` out of `root` into `dest_root`, keeping the relative path and
/// appending the conflict suffix to the file name. The synced tree itself is
/// left untouched so a later delete-pull cannot remove the copy.
pub fn preserve(root: &Path, rels: &[String], host: &str, dest_root: &Path) -> Result<Vec<PathBuf>, String> {
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut saved = Vec::new();
    for rel in rels {
        let src = root.join(rel);
        if !src.is_file() {
            continue;
        }
        let name = Path::new(rel).file_name().map_or_else(
            || "file".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let parent = Path::new(rel).parent().unwrap_or(Path::new(""));
        let dest = dest_root.join(parent).join(conflict_name(&name, host, now));
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        fs::copy(&src, &dest).map_err(|e| format!("{}: {e}", src.display()))?;
        saved.push(dest);
    }
    Ok(saved)
}

pub fn load(path: &Path) -> Manifest {
    let Ok(bytes) = fs::read(path) else {
        return Manifest::default();
    };
    postcard::from_bytes(&bytes).unwrap_or_default()
}

pub fn store(path: &Path, manifest: &Manifest) -> Result<(), String> {
    let bytes = postcard::to_allocvec(manifest).map_err(|e| e.to_string())?;
    crate::atomic_write(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_sees_size_change_and_delete() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("a.txt");
        fs::write(&file, b"one").expect("write");
        let before = scan(tmp.path());
        fs::write(&file, b"two!").expect("rewrite");
        fs::write(tmp.path().join("b.txt"), b"x").expect("write b");
        let mut changed = dirty(tmp.path(), &before);
        changed.sort();
        assert_eq!(changed, vec!["a.txt".to_string(), "b.txt".to_string()]);
        fs::remove_file(&file).expect("remove");
        let after = scan(tmp.path());
        assert_eq!(dirty(tmp.path(), &after), Vec::<String>::new());
        assert!(dirty(tmp.path(), &before).iter().any(|p| p == "a.txt"));
    }

    #[test]
    fn conflict_name_shape() {
        assert_eq!(
            conflict_name("notes.txt", "host", 10),
            "notes.txt.sinkd-conflict-host-10"
        );
    }
}
