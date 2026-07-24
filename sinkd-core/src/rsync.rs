use log::{debug, error};

use std::{
    ffi::OsStr,
    path::Path,
    process::{Command, Stdio},
};

use crate::config::RsyncOptions;
use crate::outcome::Outcome;

/// SSH user for `user@host:path` rsync sources (override with `SINKD_RSYNC_SSH_USER`).
#[must_use]
pub fn rsync_ssh_user() -> String {
    std::env::var("SINKD_RSYNC_SSH_USER").unwrap_or_else(|_| "root".to_string())
}

/// Result of probing the server mirror path before a behind-pull.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehindSourceStatus {
    /// Mirror path exists; rsync should proceed.
    Ready,
    /// Mirror path is absent (new client / first push still pending).
    Missing,
    /// Probe failed (e.g. SSH unreachable); still attempt rsync.
    Indeterminate,
}

/// Whether `source` under the sync root is ready to pull during `NotReady(Behind)`.
///
/// `server_addr` is a local sync root when it starts with `/`, otherwise a hostname.
#[must_use]
pub fn check_behind_source(server_addr: &str, source: &Path) -> BehindSourceStatus {
    if server_addr.starts_with('/') {
        if source.is_dir() {
            BehindSourceStatus::Ready
        } else {
            BehindSourceStatus::Missing
        }
    } else {
        check_remote_dir(server_addr, source)
    }
}

/// `true` when a behind-pull should no-op instead of rsync (missing mirror).
#[must_use]
pub fn skip_missing_behind_source(status: BehindSourceStatus) -> bool {
    matches!(status, BehindSourceStatus::Missing)
}

fn check_remote_dir(hostname: &str, path: &Path) -> BehindSourceStatus {
    let user = rsync_ssh_user();
    let mut cmd = Command::new("ssh");
    cmd.args(["-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=accept-new"])
        .arg(format!("{user}@{hostname}"))
        .arg("test")
        .arg("-d")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match cmd.status() {
        Ok(status) if status.success() => BehindSourceStatus::Ready,
        // Remote `test` returns 1 when the path is missing or not a directory.
        Ok(status) if status.code() == Some(1) => BehindSourceStatus::Missing,
        Ok(_) | Err(_) => BehindSourceStatus::Indeterminate,
    }
}

/// True when `spec` looks like an rsync remote source (`host:path` or `user@host:path`).
#[must_use]
pub fn is_rsync_remote_spec(spec: &str) -> bool {
    if spec.starts_with('/') {
        return false;
    }
    if let Some(idx) = spec.find(':') {
        return idx > 0;
    }
    false
}

/// Build rsync remote sources for paths on `hostname` (server pull from client over SSH).
#[must_use]
pub fn resolve_rsync_sources(hostname: &str, src_paths: &[std::path::PathBuf]) -> Vec<String> {
    let user = rsync_ssh_user();
    src_paths
        .iter()
        .map(|path| {
            let lossy = path.to_string_lossy();
            if is_rsync_remote_spec(&lossy) {
                lossy.into_owned()
            } else {
                format!("{user}@{hostname}:{}", path.display())
            }
        })
        .collect()
}

fn rsync_uses_ssh<I, S>(sources: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    sources
        .into_iter()
        .any(|s| is_rsync_remote_spec(s.as_ref()))
}

fn ssh_rsync_shell() -> String {
    std::env::var("SINKD_RSYNC_SSH_SHELL").unwrap_or_else(|_| {
        "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new".to_string()
    })
}

fn configure_rsync_cmd(cmd: &mut Command, sources: &[String]) {
    if rsync_uses_ssh(sources.iter().map(String::as_str)) {
        cmd.arg("-e").arg(ssh_rsync_shell());
    }
}

pub fn rsync<P>(
    srcs: &[P],
    dest: &P,
    rsync_options: &RsyncOptions,
    backup_dir: Option<&Path>,
) -> Outcome<()>
where
    P: AsRef<OsStr> + AsRef<Path> + std::fmt::Debug,
{
    let sources: Vec<String> = srcs
        .iter()
        .map(|p| {
            AsRef::<Path>::as_ref(p)
                .as_os_str()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let dest_str = AsRef::<Path>::as_ref(dest)
        .as_os_str()
        .to_string_lossy()
        .into_owned();
    rsync_sources(&sources, &dest_str, rsync_options, backup_dir)
}

/// Rsync with string sources (local paths or remote `user@host:path`).
pub fn rsync_sources(
    srcs: &[String],
    dest: &str,
    rsync_options: &RsyncOptions,
    backup_dir: Option<&Path>,
) -> Outcome<()> {
    if crate::test_hooks::env_flag_true("SINKD_TEST_RSYNC_FAIL") {
        error!("rsync test hook: forced failure");
        return bad!("rsync test hook: forced failure");
    }
    if let Some(delay_ms) = crate::test_hooks::env_u64("SINKD_TEST_RSYNC_DELAY_MS") {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }

    let mut cmd = Command::new("rsync");
    configure_rsync_cmd(&mut cmd, srcs);
    cmd.args(build_pull_args(rsync_options, backup_dir))
        .args(srcs)
        .arg(dest);

    let mut child = match cmd.spawn() {
        Err(e) => {
            error!("rsync spawn error: {e:#?}");
            return bad!("rsync spawn failed: {e}");
        }
        Ok(c) => c,
    };

    let status = match child.wait() {
        Err(e) => {
            error!("rsync wait error: {e:#?}");
            return bad!("rsync wait failed: {e}");
        }
        Ok(s) => s,
    };

    if !status.success() {
        error!("rsync exited with status {status}");
        return bad!("rsync failed with status {status}");
    }

    debug!("\u{1f6b0} rsync {srcs:#?} {dest:#?} backup:{backup_dir:?} \u{1f919}");
    Ok(())
}

/// Directory → directory sync (no `-R`). Used when the client reconciles from the
/// server's sync root during `NotReady(Behind)` (sync root arrives in Zenoh `dest_path`).
pub fn rsync_behind_mirror_sync(
    src_dir: &str,
    dest_dir: &str,
    rsync_options: &RsyncOptions,
    backup_dir: Option<&Path>,
) -> Outcome<()> {
    if crate::test_hooks::env_flag_true("SINKD_TEST_RSYNC_FAIL") {
        error!("rsync test hook: forced failure");
        return bad!("rsync test hook: forced failure");
    }
    if let Some(delay_ms) = crate::test_hooks::env_u64("SINKD_TEST_RSYNC_DELAY_MS") {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }

    let src_spec = src_dir.to_string();
    let mut cmd = Command::new("rsync");
    configure_rsync_cmd(&mut cmd, std::slice::from_ref(&src_spec));
    cmd.args(build_behind_pull_args(rsync_options, backup_dir))
        .arg(src_dir)
        .arg(dest_dir);

    let mut child = match cmd.spawn() {
        Err(e) => {
            error!("rsync spawn error: {e:#?}");
            return bad!("rsync spawn failed: {e}");
        }
        Ok(c) => c,
    };

    let status = match child.wait() {
        Err(e) => {
            error!("rsync wait error: {e:#?}");
            return bad!("rsync wait failed: {e}");
        }
        Ok(s) => s,
    };

    if !status.success() {
        error!("rsync exited with status {status}");
        return bad!("rsync failed with status {status}");
    }

    debug!("\u{1f6b0} rsync behind {src_dir} -> {dest_dir} backup:{backup_dir:?} \u{1f919}");
    Ok(())
}

#[must_use]
pub fn build_args(rsync_options: &RsyncOptions) -> Vec<String> {
    let mut args = vec!["-atR".to_string(), "--delete".to_string()];
    append_rsync_option_flags(&mut args, rsync_options);
    args
}

#[must_use]
fn build_behind_pull_args(
    rsync_options: &RsyncOptions,
    backup_dir: Option<&Path>,
) -> Vec<String> {
    // No `--delete`: when the server mirror for this anchor is still empty or lagging
    // (e.g. a second client joins while global generation is already ahead), a delete
    // pass would wipe the client's pending tree before the first successful push lands.
    let mut args = vec!["-a".to_string()];
    append_rsync_option_flags(&mut args, rsync_options);
    if let Some(dir) = backup_dir {
        args.push("--backup".to_string());
        args.push(format!(
            "--backup-dir={}",
            dir.as_os_str().to_string_lossy()
        ));
    }
    args
}

fn append_rsync_option_flags(args: &mut Vec<String>, rsync_options: &RsyncOptions) {
    if rsync_options.checksum.unwrap_or(false) {
        args.push("--checksum".to_string());
    }
    if rsync_options.compress.unwrap_or(false) {
        args.push("--compress".to_string());
    }
    if let Some(bwlimit) = &rsync_options.bwlimit {
        args.push(format!("--bwlimit={bwlimit}"));
    }
    if rsync_options.partial.unwrap_or(false) {
        args.push("--partial".to_string());
    }
    if rsync_options.delete_excluded.unwrap_or(false) {
        args.push("--delete-excluded".to_string());
    }
    if let Some(max_size) = &rsync_options.max_size {
        args.push(format!("--max-size={max_size}"));
    }
    if let Some(min_size) = &rsync_options.min_size {
        args.push(format!("--min-size={min_size}"));
    }
    if rsync_options.ignore_existing.unwrap_or(false) {
        args.push("--ignore-existing".to_string());
    }
    if rsync_options.size_only.unwrap_or(false) {
        args.push("--size-only".to_string());
    }
    if rsync_options.stats.unwrap_or(false) {
        args.push("--stats".to_string());
    }
}

/// Full rsync argv for a pull (baseline flags plus optional `--backup` / `--backup-dir`).
#[must_use]
pub fn build_pull_args(rsync_options: &RsyncOptions, backup_dir: Option<&Path>) -> Vec<String> {
    let mut args = build_args(rsync_options);
    if let Some(dir) = backup_dir {
        args.push("--backup".to_string());
        args.push(format!(
            "--backup-dir={}",
            dir.as_os_str().to_string_lossy()
        ));
    }
    args
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::config::RsyncOptions;

    use super::{
        build_args, build_pull_args, check_behind_source, is_rsync_remote_spec,
        resolve_rsync_sources, skip_missing_behind_source, BehindSourceStatus,
    };

    #[test]
    fn is_rsync_remote_spec_detects_host_paths() {
        assert!(is_rsync_remote_spec("server:/srv/sinkd/"));
        assert!(is_rsync_remote_spec("root@client-alice:/watch/"));
        assert!(!is_rsync_remote_spec("/watch/alice"));
    }

    #[test]
    fn check_behind_source_local_missing_vs_ready() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("nope");
        assert_eq!(
            check_behind_source("/", &missing),
            BehindSourceStatus::Missing
        );
        let ready = tmp.path().join("mirror");
        std::fs::create_dir_all(&ready).expect("mkdir");
        assert_eq!(check_behind_source("/", &ready), BehindSourceStatus::Ready);
    }

    #[test]
    fn skip_missing_behind_source_only_for_missing() {
        assert!(skip_missing_behind_source(BehindSourceStatus::Missing));
        assert!(!skip_missing_behind_source(BehindSourceStatus::Ready));
        assert!(!skip_missing_behind_source(
            BehindSourceStatus::Indeterminate
        ));
    }

    #[test]
    fn resolve_rsync_sources_builds_ssh_urls() {
        let paths = vec![PathBuf::from("/watch_alice/file.txt")];
        let out = resolve_rsync_sources("client-alice", &paths);
        assert_eq!(out, vec!["root@client-alice:/watch_alice/file.txt".to_string()]);
    }

    #[test]
    fn build_args_keeps_baseline_defaults() {
        let args = build_args(&RsyncOptions::default());
        assert_eq!(args, vec!["-atR", "--delete"]);
    }

    #[test]
    fn build_args_appends_allowlisted_flags() {
        let options = RsyncOptions {
            checksum: Some(true),
            compress: Some(true),
            bwlimit: Some("2m".to_string()),
            partial: Some(true),
            delete_excluded: Some(true),
            max_size: Some("10m".to_string()),
            min_size: Some("1k".to_string()),
            ignore_existing: Some(true),
            size_only: Some(true),
            stats: Some(true),
            ..Default::default()
        };
        let args = build_args(&options);
        assert_eq!(
            args,
            vec![
                "-atR",
                "--delete",
                "--checksum",
                "--compress",
                "--bwlimit=2m",
                "--partial",
                "--delete-excluded",
                "--max-size=10m",
                "--min-size=1k",
                "--ignore-existing",
                "--size-only",
                "--stats",
            ]
        );
    }

    #[test]
    fn build_pull_args_matches_build_args_without_backup() {
        let options = RsyncOptions::default();
        assert_eq!(build_pull_args(&options, None), build_args(&options));
    }

    #[test]
    fn build_pull_args_appends_backup_flags_with_absolute_dir() {
        let options = RsyncOptions::default();
        let backup = Path::new("/tmp/sinkd/client/behind_backups/0");
        let args = build_pull_args(&options, Some(backup));
        assert!(
            args.ends_with(&[
                "--backup".to_string(),
                "--backup-dir=/tmp/sinkd/client/behind_backups/0".to_string(),
            ]),
            "args={args:?}"
        );
    }
}
