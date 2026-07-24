//! Deterministic directory manifests for scenario tree comparison.

use std::{
    fmt::Write as _,
    fs,
    io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

const EXCLUDED_NAMES: &[&str] = &[".events_done"];

fn is_excluded_file_name(name: &str) -> bool {
    EXCLUDED_NAMES.contains(&name) || name.ends_with(".partial")
}

/// Walk `dir` and emit sorted lines: `<sha256>  ./relative/path`
pub fn generate(dir: &Path) -> Result<String, String> {
    if !dir.is_dir() {
        return Err(format!("manifest: not a directory: {}", dir.display()));
    }
    let base = dir
        .canonicalize()
        .map_err(|e| format!("manifest: canonicalize '{}': {e}", dir.display()))?;
    let mut entries = Vec::new();
    collect_files(&base, &base, &mut entries)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    for (rel, path) in entries {
        let hash = hash_file(&path)?;
        out.push_str(&hash);
        out.push_str("  ./");
        out.push_str(&rel);
        out.push('\n');
    }
    Ok(out)
}

fn collect_files(base: &Path, current: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|e| format!("manifest: read '{}': {e}", current.display()))? {
        let entry = entry.map_err(|e| format!("manifest: dir entry: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, out)?;
        } else if path.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_excluded_file_name(name) {
                continue;
            }
            let rel = path
                .strip_prefix(base)
                .map_err(|e| format!("manifest: strip prefix: {e}"))?
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, path));
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("manifest: read '{}': {e}", path.display()))?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{digest:x}"))
}

/// Compare two manifest files (or manifest text). Returns Ok on match.
pub fn compare_files(client: &Path, server: &Path) -> Result<(), String> {
    let client_text = fs::read_to_string(client)
        .map_err(|e| format!("manifest: read client '{}': {e}", client.display()))?;
    let server_text = fs::read_to_string(server)
        .map_err(|e| format!("manifest: read server '{}': {e}", server.display()))?;
    compare_text(&client_text, &server_text)
}

pub fn compare_text(client: &str, server: &str) -> Result<(), String> {
    if client == server {
        Ok(())
    } else {
        Err(diff_summary(client, server))
    }
}

fn diff_summary(client: &str, server: &str) -> String {
    let client_lines: Vec<_> = client.lines().collect();
    let server_lines: Vec<_> = server.lines().collect();
    let mut msg = format!(
        "manifest mismatch (client {} lines, server {} lines)",
        client_lines.len(),
        server_lines.len()
    );
    let max = client_lines.len().max(server_lines.len()).min(20);
    for i in 0..max {
        let c = client_lines.get(i).copied().unwrap_or("<missing>");
        let s = server_lines.get(i).copied().unwrap_or("<missing>");
        if c != s {
            let _ = writeln!(msg, "  line {} client: {c}", i + 1);
            let _ = writeln!(msg, "  line {} server: {s}", i + 1);
        }
    }
    if client_lines.len().max(server_lines.len()) > max {
        msg.push_str("\n  ...");
    }
    msg
}

pub fn write_manifest(dir: &Path, output: &Path) -> Result<(), String> {
    let text = generate(dir)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("manifest: mkdir '{}': {e}", parent.display()))?;
    }
    fs::write(output, text).map_err(|e| format!("manifest: write '{}': {e}", output.display()))
}

pub fn generate_to_stdout(dir: &Path) -> io::Result<()> {
    let text = generate(dir).map_err(io::Error::other)?;
    print!("{text}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn generate_sorts_and_hashes_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("b.txt"), "two").expect("write");
        fs::write(dir.path().join("a.txt"), "one").expect("write");
        fs::write(dir.path().join(".events_done"), "sentinel").expect("write");
        let manifest = generate(dir.path()).expect("generate");
        let lines: Vec<_> = manifest.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("  ./a.txt"));
        assert!(lines[1].ends_with("  ./b.txt"));
    }

    #[test]
    fn compare_text_detects_drift() {
        assert!(compare_text("a\n", "a\n").is_ok());
        assert!(compare_text("a\n", "b\n").is_err());
    }
}
