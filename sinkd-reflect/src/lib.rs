//! Safe `libreflect` jobs.
//!
//! shortcut: the filter hook inside libreflect is process-global, so every job
//! takes a process-wide lock. Drop the lock if a future libreflect makes the
//! hook job-local.

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use sinkd_core::Rules;
use sinkd_reflect_sys as sys;

const OK: i32 = sys::reflect_code_REFLECT_OK.cast_signed();
const IDLE: i32 = sys::reflect_code_REFLECT_JOB_IDLE.cast_signed();
const RUNNING: i32 = sys::reflect_code_REFLECT_JOB_RUNNING.cast_signed();

static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static INIT: OnceLock<Result<(), String>> = OnceLock::new();

fn lock() -> MutexGuard<'static, ()> {
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn ensure_init() -> Result<(), String> {
    INIT.get_or_init(|| {
        let result = unsafe { sys::reflect_lib_init() };
        if code_of(result.code) == OK {
            Ok(())
        } else {
            Err(format!(
                "reflect_lib_init {} {}",
                code_of(result.code),
                message_of(&result)
            ))
        }
    })
    .clone()
}

fn code_of(code: sys::reflect_code) -> i32 {
    code.cast_signed()
}

fn message_of(result: &sys::reflect_result) -> String {
    if result.message.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(result.message) }
        .to_string_lossy()
        .into_owned()
}

fn explain(result: &sys::reflect_result, allowed: &[i32]) -> Result<(), String> {
    let code = code_of(result.code);
    if allowed.contains(&code) {
        return Ok(());
    }
    Err(format!("reflect {code}: {}", message_of(result)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub files: i32,
    pub bytes: i64,
}

fn stats_of(result: &sys::reflect_result) -> Stats {
    if result.stats.is_null() {
        return Stats { files: 0, bytes: 0 };
    }
    let stats = unsafe { &*result.stats };
    Stats {
        files: stats.xferred_files,
        bytes: stats.total_transferred_size,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Opts {
    pub delete: bool,
    pub update_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// This process reads the tree and writes the protocol.
    Send,
    /// This process reads the protocol and writes the tree.
    Receive,
}

static EXTRA: std::sync::Mutex<Option<fn(&str, bool) -> bool>> = std::sync::Mutex::new(None);

/// `skip` returns true when this job should not copy the path. One job at a time.
pub fn set_extra_filter(skip: Option<fn(&str, bool) -> bool>) {
    *EXTRA.lock().unwrap_or_else(|err| err.into_inner()) = skip;
}

extern "C" fn on_filter(_user: *mut std::ffi::c_void, path: *const std::ffi::c_char, is_dir: i32) -> i32 {
    if path.is_null() {
        return 1;
    }
    let text = unsafe { CStr::from_ptr(path) }.to_string_lossy();
    if text.contains(".sinkd-conflict-") {
        return 0;
    }
    let skip = *EXTRA.lock().unwrap_or_else(|err| err.into_inner());
    if skip.is_some_and(|skip| skip(&text, is_dir != 0)) {
        return 0;
    }
    1
}

pub struct Job {
    raw: sys::reflect_job,
    // Held so the C strings outlive reflect_job_cleanup.
    #[allow(dead_code)]
    kept: Vec<CString>,
    ready: bool,
}

impl Job {
    fn blank() -> Self {
        Self {
            raw: sys::reflect_job { words: [0; 8] },
            kept: Vec::new(),
            ready: false,
        }
    }

    fn opts(options: Opts) -> sys::reflect_opts {
        let mut opts = unsafe { sys::reflect_default_opts() };
        opts.run.delete_mode = i32::from(options.delete);
        opts.run.update_only = i32::from(options.update_only);
        opts.tree.recurse = sys::reflect_recurse_REFLECT_RECURSE_ON;
        opts.filters.honor_gitignore = 0;
        opts
    }

    fn arm_callbacks(&mut self) -> Result<(), String> {
        explain(
            &unsafe {
                sys::reflect_job_set_filter(&raw mut self.raw, Some(on_filter), std::ptr::null_mut())
            },
            &[OK, IDLE, RUNNING],
        )
    }

    /// `sources` are local trees. A directory gets a trailing slash so its contents land in `dest`.
    pub fn open_paths(options: Opts, sources: &[impl AsRef<Path>], dest: &Path) -> Result<Self, String> {
        ensure_init()?;
        let mut job = Self::blank();
        let src = c_sources(sources)?;
        let dst = CString::new(dest_string(dest)).map_err(|e| e.to_string())?;
        let ptrs: Vec<*const std::ffi::c_char> = src.iter().map(|s| s.as_ptr()).collect();
        let opts = Self::opts(options);
        let result = unsafe {
            sys::reflect_job_init(
                &raw mut job.raw,
                &raw const opts,
                ptrs.as_ptr(),
                ptrs.len(),
                dst.as_ptr(),
            )
        };
        explain(&result, &[IDLE])?;
        job.kept = src;
        job.kept.push(dst);
        job.ready = true;
        job.arm_callbacks()?;
        Ok(job)
    }

    pub fn open_fd(
        options: Opts,
        side: Side,
        dir: &Path,
        in_fd: i32,
        out_fd: i32,
    ) -> Result<Self, String> {
        ensure_init()?;
        let mut job = Self::blank();
        let opts = Self::opts(options);
        let result = match side {
            Side::Send => {
                let src = c_sources(std::slice::from_ref(&dir))?;
                let ptrs: Vec<*const std::ffi::c_char> = src.iter().map(|s| s.as_ptr()).collect();
                let result = unsafe {
                    sys::reflect_job_init_fd(
                        &raw mut job.raw,
                        &raw const opts,
                        sys::reflect_fd_role_REFLECT_FD_SEND,
                        ptrs.as_ptr(),
                        ptrs.len(),
                        std::ptr::null(),
                        in_fd,
                        out_fd,
                    )
                };
                job.kept = src;
                result
            }
            Side::Receive => {
                let dst = CString::new(dir.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
                let result = unsafe {
                    sys::reflect_job_init_fd(
                        &raw mut job.raw,
                        &raw const opts,
                        sys::reflect_fd_role_REFLECT_FD_RECEIVE,
                        std::ptr::null(),
                        0,
                        dst.as_ptr(),
                        in_fd,
                        out_fd,
                    )
                };
                job.kept.push(dst);
                result
            }
        };
        explain(&result, &[IDLE])?;
        job.ready = true;
        job.arm_callbacks()?;
        Ok(job)
    }

    pub fn run(&mut self) -> Result<Stats, String> {
        let _guard = lock();
        let result = unsafe { sys::reflect_job_run(&raw mut self.raw) };
        explain(&result, &[OK])?;
        Ok(stats_of(&result))
    }

}

impl Drop for Job {
    fn drop(&mut self) {
        if self.ready {
            unsafe { sys::reflect_job_cleanup(&raw mut self.raw) };
        }
    }
}

fn dest_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn c_sources(sources: &[impl AsRef<Path>]) -> Result<Vec<CString>, String> {
    sources
        .iter()
        .map(|path| {
            let path = path.as_ref();
            let mut text = path.to_string_lossy().into_owned();
            if path.is_dir() && !text.ends_with('/') {
                text.push('/');
            }
            CString::new(text).map_err(|e| e.to_string())
        })
        .collect()
}

pub fn sync_paths(options: Opts, sources: &[impl AsRef<Path>], dest: &Path) -> Result<Stats, String> {
    let mut job = Job::open_paths(options, sources, dest)?;
    job.run()
}

pub fn sync_socket(
    socket: &std::net::TcpStream,
    side: Side,
    dir: &Path,
    options: Opts,
) -> Result<Stats, String> {
    // libreflect's fd receiver never removes dest files the sender dropped.
    let keep = if options.delete {
        exchange_keep(socket, side, dir)?
    } else {
        None
    };
    let in_fd = socket.as_raw_fd();
    let out_fd = unsafe { libc::dup(in_fd) };
    if out_fd < 0 {
        return Err(format!("dup failed: {}", std::io::Error::last_os_error()));
    }
    let result = (|| {
        let mut job = Job::open_fd(options, side, dir, in_fd, out_fd)?;
        job.run()
    })();
    unsafe { libc::close(out_fd) };
    let stats = result?;
    if let Some(keep) = keep {
        prune(dir, &keep)?;
    }
    Ok(stats)
}

const MAX_KEEP: u32 = 1_000_000;
const MAX_PATH: u32 = 4096;

fn exchange_keep(socket: &std::net::TcpStream, side: Side, dir: &Path) -> Result<Option<Vec<String>>, String> {
    match side {
        Side::Send => {
            write_keep(socket, &list_files(dir)?)?;
            Ok(None)
        }
        Side::Receive => Ok(Some(read_keep(socket)?)),
    }
}

fn list_files(root: &Path) -> Result<Vec<String>, String> {
    let rules = ignore_rules();
    let mut out = Vec::new();
    walk_files(root, root, rules.as_ref(), &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_files(root: &Path, dir: &Path, rules: Option<&Rules>, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if name.to_string_lossy().contains(".sinkd-conflict-") {
            continue;
        }
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let Ok(rel) = path.strip_prefix(root) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if kind.is_dir() {
            if rules.is_some_and(|rules| rules.ignored(&rel, true) && !rules.descend(&rel)) {
                continue;
            }
            walk_files(root, &path, rules, out)?;
            continue;
        }
        if !kind.is_file() && !kind.is_symlink() {
            continue;
        }
        if rules.is_some_and(|rules| rules.ignored(&rel, false)) {
            continue;
        }
        out.push(rel);
    }
    Ok(())
}

fn write_keep(mut w: impl Write, keep: &[String]) -> Result<(), String> {
    let count = u32::try_from(keep.len()).map_err(|_| "keep list too large".to_string())?;
    if count > MAX_KEEP {
        return Err("keep list too large".into());
    }
    w.write_all(&count.to_be_bytes()).map_err(|e| e.to_string())?;
    for path in keep {
        let len = u32::try_from(path.len()).map_err(|_| "path too long".to_string())?;
        if len > MAX_PATH {
            return Err(format!("path too long: {path}"));
        }
        w.write_all(&len.to_be_bytes()).map_err(|e| e.to_string())?;
        w.write_all(path.as_bytes()).map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())
}

fn read_keep(mut r: impl Read) -> Result<Vec<String>, String> {
    let mut count_buf = [0u8; 4];
    r.read_exact(&mut count_buf).map_err(|e| format!("keep list: {e}"))?;
    let count = u32::from_be_bytes(count_buf);
    if count > MAX_KEEP {
        return Err("keep list too large".into());
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut len_buf = [0u8; 4];
        r.read_exact(&mut len_buf).map_err(|e| format!("keep list: {e}"))?;
        let len = u32::from_be_bytes(len_buf);
        if len > MAX_PATH {
            return Err("path too long".into());
        }
        let mut buf = vec![0u8; len as usize];
        r.read_exact(&mut buf).map_err(|e| format!("keep list: {e}"))?;
        let path = String::from_utf8(buf).map_err(|e| e.to_string())?;
        out.push(path);
    }
    Ok(out)
}

fn prune(root: &Path, keep: &[String]) -> Result<(), String> {
    let keep: HashSet<&str> = keep.iter().map(String::as_str).collect();
    let mut extra = Vec::new();
    extra_files(root, root, &keep, &mut extra)?;
    for path in extra {
        std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

fn extra_files(root: &Path, dir: &Path, keep: &HashSet<&str>, extra: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if name.to_string_lossy().contains(".sinkd-conflict-") {
            continue;
        }
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            extra_files(root, &path, keep, extra)?;
            continue;
        }
        let rel = path.strip_prefix(root).map_err(|e| e.to_string())?;
        let rel = rel.to_string_lossy().replace('\\', "/");
        if !keep.contains(rel.as_str()) {
            extra.push(path);
        }
    }
    Ok(())
}

/// One-shot SSH transfer. `remote` is `user@host:/path`.
pub fn sync_ssh(local: &Path, remote: &str, options: Opts) -> Result<Stats, String> {
    sync_paths(options, &[local], Path::new(remote))
}

static IGNORE_JOB: std::sync::Mutex<Option<IgnoreActive>> = std::sync::Mutex::new(None);

#[derive(Clone)]
struct IgnoreActive {
    root: PathBuf,
    rules: Rules,
}

/// Arms libreflect's global filter for one blocking sync job.
pub struct IgnoreGuard;

impl IgnoreGuard {
    pub fn arm(root: PathBuf, rules: Rules) -> Self {
        *IGNORE_JOB.lock().unwrap_or_else(|err| err.into_inner()) = Some(IgnoreActive { root, rules });
        set_extra_filter(Some(ignore_skip));
        Self
    }
}

impl Drop for IgnoreGuard {
    fn drop(&mut self) {
        *IGNORE_JOB.lock().unwrap_or_else(|err| err.into_inner()) = None;
        set_extra_filter(None);
    }
}

fn ignore_rules() -> Option<Rules> {
    IGNORE_JOB
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .as_ref()
        .map(|active| active.rules.clone())
}

fn ignore_skip(path: &str, is_dir: bool) -> bool {
    let guard = IGNORE_JOB.lock().unwrap_or_else(|err| err.into_inner());
    let Some(active) = guard.as_ref() else {
        return false;
    };
    let rel = rel_for_filter(&active.root, path);
    active.rules.ignored(&rel, is_dir)
}

fn rel_for_filter(root: &Path, path: &str) -> String {
    let text = path.trim_start_matches(['/', '\\']).replace('\\', "/");
    let as_path = Path::new(&text);
    if let Ok(rel) = as_path.strip_prefix(root) {
        return rel.to_string_lossy().replace('\\', "/");
    }
    text
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn local_mirror_copies_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).expect("mkdir src");
        fs::create_dir_all(&dst).expect("mkdir dst");
        fs::write(src.join("a.txt"), b"hi").expect("write");
        let stats = sync_paths(Opts { delete: true, ..Opts::default() }, &[src], &dst).expect("sync");
        assert_eq!(fs::read(dst.join("a.txt")).expect("read"), b"hi");
        assert!(stats.files >= 1);
    }

    #[test]
    fn list_files_honors_ignore_guard() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        fs::create_dir_all(root.join("skip")).expect("mkdir");
        fs::create_dir_all(root.join("keep")).expect("mkdir");
        fs::write(root.join("skip/hidden.txt"), b"x").expect("write");
        fs::write(root.join("keep/visible.txt"), b"y").expect("write");
        let rules = sinkd_core::Rules::load(root, &["skip/".to_string()]);
        let _guard = IgnoreGuard::arm(root.to_path_buf(), rules);
        let listed = list_files(root).expect("list");
        assert!(listed.iter().any(|p| p == "keep/visible.txt"));
        assert!(!listed.iter().any(|p| p.contains("hidden")));
    }

    #[test]
    fn delete_keep_list_drops_the_extra_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("sub")).expect("mkdir src");
        fs::create_dir_all(&dst).expect("mkdir dst");
        fs::create_dir_all(dst.join("sub")).expect("mkdir dst sub");
        fs::write(src.join("sub/b.txt"), b"stay").expect("write b");
        fs::write(dst.join("a.txt"), b"gone").expect("write a");
        fs::write(dst.join("sub/b.txt"), b"stay").expect("write kept");
        let listed = list_files(&src).expect("list");
        let mut buf = Vec::new();
        write_keep(&mut buf, &listed).expect("write");
        let keep = read_keep(buf.as_slice()).expect("read");
        assert_eq!(keep, vec!["sub/b.txt".to_string()]);
        prune(&dst, &keep).expect("prune");
        assert!(!dst.join("a.txt").exists());
        assert_eq!(fs::read(dst.join("sub/b.txt")).expect("kept"), b"stay");
    }
}
