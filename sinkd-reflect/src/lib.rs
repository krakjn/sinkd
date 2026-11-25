//! Safe `libreflect` jobs.
//!
//! Each job owns its ignore `Rules` and hands them to libreflect as the filter's
//! `user` pointer. Filters are job-local, so jobs run concurrently without a lock.
//! On SEND the filter skips transfer; on RECEIVE it protects local ignored paths
//! from `delete`.

use std::ffi::{c_char, c_void, CStr, CString};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::OnceLock;

use sinkd_core::Rules;
use sinkd_reflect_sys as sys;

const OK: i32 = sys::reflect_code_REFLECT_OK.cast_signed();
const IDLE: i32 = sys::reflect_code_REFLECT_JOB_IDLE.cast_signed();
const RUNNING: i32 = sys::reflect_code_REFLECT_JOB_RUNNING.cast_signed();
const DEL_LIMIT: i32 = sys::reflect_code_REFLECT_ERR_DEL_LIMIT.cast_signed();

// The filter runs on several libreflect threads at once.
const _: () = {
    const fn sync<T: Send + Sync>() {}
    sync::<Rules>();
};

static INIT: OnceLock<Result<(), String>> = OnceLock::new();

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
    if code == DEL_LIMIT {
        return Err(format!("reflect: delete limit reached: {}", message_of(result)));
    }
    Err(format!("reflect {code}: {}", message_of(result)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub files: i32,
    pub bytes: i64,
    pub deleted: i32,
}

fn stats_of(result: &sys::reflect_result) -> Stats {
    if result.stats.is_null() {
        return Stats { files: 0, bytes: 0, deleted: 0 };
    }
    let stats = unsafe { &*result.stats };
    Stats {
        files: stats.xferred_files,
        bytes: stats.total_transferred_size,
        deleted: stats.deleted_files,
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

extern "C" fn on_filter(user: *mut c_void, path: *const c_char, is_dir: i32) -> i32 {
    if path.is_null() {
        return 1;
    }
    let text = unsafe { CStr::from_ptr(path) }.to_string_lossy();
    let rel = text.trim_start_matches('/');
    if rel.contains(".sinkd-conflict-") {
        return 0;
    }
    if user.is_null() {
        return 1;
    }
    let rules = unsafe { &*(user as *const Rules) };
    // libreflect never descends an excluded directory, so one a later `!` reaches into must stay.
    let keep = if is_dir != 0 { rules.descend(rel) } else { !rules.ignored(rel, false) };
    i32::from(keep)
}

pub struct Job {
    raw: sys::reflect_job,
    // Held so the C strings and the filter's rules outlive reflect_job_cleanup.
    #[allow(dead_code)]
    kept: Vec<CString>,
    rules: Box<Rules>,
    ready: bool,
}

impl Job {
    fn blank(rules: Rules) -> Self {
        Self {
            raw: sys::reflect_job { words: [0; 8] },
            kept: Vec::new(),
            rules: Box::new(rules),
            ready: false,
        }
    }

    fn opts(options: Opts, gitignore: bool) -> sys::reflect_opts {
        let mut opts = unsafe { sys::reflect_default_opts() };
        opts.run.delete_mode = i32::from(options.delete);
        opts.run.update_only = i32::from(options.update_only);
        opts.tree.recurse = sys::reflect_recurse_REFLECT_RECURSE_ON;
        opts.filters.honor_gitignore = i32::from(gitignore);
        opts
    }

    fn arm_callbacks(&mut self) -> Result<(), String> {
        let user = std::ptr::from_ref::<Rules>(&self.rules).cast_mut().cast::<c_void>();
        explain(
            &unsafe { sys::reflect_job_set_filter(&raw mut self.raw, Some(on_filter), user) },
            &[OK, IDLE, RUNNING],
        )
    }

    /// `sources` are local trees. A directory gets a trailing slash so its contents land in `dest`.
    pub fn open_paths(
        options: Opts,
        sources: &[impl AsRef<Path>],
        dest: &Path,
        rules: Rules,
    ) -> Result<Self, String> {
        Self::open_paths_with(options, sources, dest, rules, false)
    }

    fn open_paths_with(
        options: Opts,
        sources: &[impl AsRef<Path>],
        dest: &Path,
        rules: Rules,
        gitignore: bool,
    ) -> Result<Self, String> {
        ensure_init()?;
        let mut job = Self::blank(rules);
        let src = c_sources(sources)?;
        let dst = CString::new(dest.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let ptrs: Vec<*const c_char> = src.iter().map(|s| s.as_ptr()).collect();
        let opts = Self::opts(options, gitignore);
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
        rules: Rules,
    ) -> Result<Self, String> {
        ensure_init()?;
        let mut job = Self::blank(rules);
        let opts = Self::opts(options, false);
        let result = match side {
            Side::Send => {
                let src = c_sources(std::slice::from_ref(&dir))?;
                let ptrs: Vec<*const c_char> = src.iter().map(|s| s.as_ptr()).collect();
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

pub fn sync_paths(
    options: Opts,
    sources: &[impl AsRef<Path>],
    dest: &Path,
    rules: Rules,
) -> Result<Stats, String> {
    Job::open_paths(options, sources, dest, rules)?.run()
}

pub fn sync_socket(
    socket: &std::net::TcpStream,
    side: Side,
    dir: &Path,
    options: Opts,
    rules: Rules,
) -> Result<Stats, String> {
    let in_fd = socket.as_raw_fd();
    let out_fd = unsafe { libc::dup(in_fd) };
    if out_fd < 0 {
        return Err(format!("dup failed: {}", std::io::Error::last_os_error()));
    }
    let result = Job::open_fd(options, side, dir, in_fd, out_fd, rules).and_then(|mut job| job.run());
    unsafe { libc::close(out_fd) };
    result
}

/// One-shot SSH transfer. `remote` is `user@host:/path`.
///
/// The filter callback never reaches the remote receiver, so `.gitignore` is also
/// handed to libreflect, which sends it on the wire and protects remote ignored
/// paths from `delete`. That also skips `.git/`.
pub fn sync_ssh(local: &Path, remote: &str, options: Opts, rules: Rules) -> Result<Stats, String> {
    Job::open_paths_with(options, &[local], Path::new(remote), rules, true)?.run()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::thread;

    use super::*;

    fn rules(root: &Path, lines: &[&str]) -> Rules {
        let lines: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
        Rules::load(root, &lines)
    }

    fn write(path: PathBuf, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    /// Sender pushes `src` into `dst` over loopback TCP, both jobs in this process.
    fn fd_sync(src: &Path, dst: &Path, send_rules: Rules, recv_rules: Rules, delete: bool) -> (Stats, Stats) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let opts = Opts { delete, update_only: false };
        let dst = dst.to_path_buf();
        let recv = thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            sync_socket(&sock, Side::Receive, &dst, opts, recv_rules)
        });
        let sock = TcpStream::connect(addr).unwrap();
        let sent = sync_socket(&sock, Side::Send, src, opts, send_rules).expect("send");
        drop(sock);
        let got = recv.join().unwrap().expect("receive");
        (sent, got)
    }

    #[test]
    fn local_mirror_copies_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        write(src.join("a.txt"), "hi");
        fs::create_dir_all(&dst).unwrap();
        let stats = sync_paths(Opts { delete: true, ..Opts::default() }, &[&src], &dst, Rules::default()).unwrap();
        assert_eq!(fs::read(dst.join("a.txt")).unwrap(), b"hi");
        assert!(stats.files >= 1);
    }

    #[test]
    fn local_delete_protects_ignored_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        write(src.join("keep.txt"), "k");
        write(src.join("build/y"), "y");
        write(dst.join("gone.txt"), "g");
        write(dst.join("build/x"), "x");
        let rules = rules(&src, &["build/"]);
        let stats = sync_paths(Opts { delete: true, ..Opts::default() }, &[&src], &dst, rules).unwrap();
        assert!(dst.join("keep.txt").exists());
        assert!(!dst.join("gone.txt").exists(), "dest-only file not deleted");
        assert!(dst.join("build/x").exists(), "ignored dest file was deleted");
        assert!(!dst.join("build/y").exists(), "ignored src file was sent");
        assert!(stats.deleted >= 1);
    }

    #[test]
    fn fd_pair_delete_in_one_process() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        write(src.join("sub/b.txt"), "stay");
        write(src.join("node_modules/pkg.js"), "skip");
        write(dst.join("a.txt"), "gone");
        write(dst.join("sub/b.txt"), "old");
        write(dst.join("node_modules/local.js"), "mine");
        let (_, got) = fd_sync(
            &src,
            &dst,
            rules(&src, &["node_modules/"]),
            rules(&dst, &["node_modules/"]),
            true,
        );
        assert!(!dst.join("a.txt").exists(), "fd receiver did not delete");
        assert_eq!(fs::read_to_string(dst.join("sub/b.txt")).unwrap(), "stay");
        assert!(dst.join("node_modules/local.js").exists(), "receiver ignored file was deleted");
        assert!(!dst.join("node_modules/pkg.js").exists(), "sender ignored file was sent");
        assert!(got.deleted >= 1, "deleted stat: {got:?}");
    }

    #[test]
    fn negation_reincludes_child() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        write(src.join("something/thing/a.txt"), "a");
        write(src.join("something/other/b.txt"), "b");
        write(src.join("top.txt"), "t");
        write(dst.join("something/local.txt"), "mine");
        write(dst.join("something/thing/stale.txt"), "old");
        let rules = rules(&src, &["something/", "!something/thing/"]);
        sync_paths(Opts { delete: true, ..Opts::default() }, &[&src], &dst, rules).unwrap();
        assert!(dst.join("top.txt").exists());
        assert!(dst.join("something/local.txt").exists(), "ignored dest file under descended dir deleted");
        assert!(!dst.join("something/thing/stale.txt").exists(), "re-included stale file kept");
        assert!(dst.join("something/thing/a.txt").exists(), "re-included child missing");
        assert!(!dst.join("something/other/b.txt").exists(), "excluded sibling sent");
    }

    #[test]
    fn concurrent_jobs_keep_own_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_path_buf();
        let runs: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|skip| {
                let base = base.clone();
                thread::spawn(move || {
                    let src = base.join(format!("src-{skip}"));
                    let dst = base.join(format!("dst-{skip}"));
                    for dir in ["a", "b"] {
                        for i in 0..50 {
                            write(src.join(format!("{dir}/f{i}.txt")), "x");
                        }
                    }
                    fs::create_dir_all(&dst).unwrap();
                    let pat = format!("{skip}/");
                    fd_sync(&src, &dst, rules(&src, &[&pat]), rules(&dst, &[&pat]), true);
                    (skip, dst)
                })
            })
            .collect();
        for run in runs {
            let (skip, dst) = run.join().unwrap();
            let kept = if skip == "a" { "b" } else { "a" };
            assert!(!dst.join(skip).exists(), "{skip}/ leaked into its own job");
            for i in 0..50 {
                assert!(dst.join(format!("{kept}/f{i}.txt")).exists(), "{kept}/f{i} missing in job {skip}");
            }
        }
    }
}
