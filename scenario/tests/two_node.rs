//! Server and clients run as separate processes, the way they ship.

use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct Stop(Child);

impl Drop for Stop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn target_triple() -> &'static str {
    if cfg!(all(target_arch = "x86_64", target_os = "linux", target_env = "musl")) {
        "x86_64-unknown-linux-musl"
    } else if cfg!(all(target_arch = "aarch64", target_os = "linux", target_env = "musl")) {
        "aarch64-unknown-linux-musl"
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        "x86_64-apple-darwin"
    } else {
        panic!("no target triple for this build")
    }
}

fn bin(name: &str) -> PathBuf {
    // .cargo/config.toml defaults to x86_64 musl. A bare cargo build on the
    // macOS runner looks for that target and fails.
    let triple = target_triple();
    let mut path = PathBuf::from(std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        root.pop();
        root.join("target").to_string_lossy().into_owned()
    }));
    path.extend([triple, "debug", name]);
    if !path.exists() {
        let status = Command::new("cargo")
            .args(["build", "-p", "sinkd", "-p", "sinkd-srv", "--target", triple])
            .status()
            .expect("cargo build");
        assert!(status.success(), "cargo build sinkd bins");
    }
    assert!(path.exists(), "missing {}", path.display());
    path
}

fn spawn(bin: &Path, args: &[&str], log: &Path) -> Stop {
    let file = fs::OpenOptions::new().create(true).append(true).open(log).unwrap();
    let child = Command::new(bin)
        .args(args)
        .stdout(Stdio::from(file.try_clone().unwrap()))
        .stderr(Stdio::from(file))
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", bin.display()));
    Stop(child)
}

fn count_lines(path: &Path, needle: &str) -> usize {
    let Ok(file) = fs::File::open(path) else { return 0 };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| line.contains(needle))
        .count()
}

fn wait_lines(path: &Path, needle: &str, at_least: usize, limit: Duration) {
    let start = Instant::now();
    while start.elapsed() < limit {
        if count_lines(path, needle) >= at_least {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {needle} in {}:\n{}", path.display(), fs::read_to_string(path).unwrap_or_default());
}

fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().and_then(|s| s.to_str()) == Some(name) && path.is_file() {
                return Some(path);
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    None
}

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

struct Hub {
    root: tempfile::TempDir,
    sync: PathBuf,
    log: PathBuf,
    zenoh: u16,
    pair: u16,
    code: String,
    cli: PathBuf,
    _server: Stop,
}

struct Client {
    log: PathBuf,
    args: Vec<String>,
    cli: PathBuf,
    child: Option<Stop>,
}

impl Hub {
    /// Uses `base`, `base + 1`, and `base + 2` for zenoh, data, and pair.
    fn start(base: u16, code: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let sync = root.path().join("sync");
        let log = root.path().join("srv.log");
        let (zenoh, data, pair) = (base, base + 1, base + 2);
        let p = |name: &str| root.path().join(name).to_string_lossy().into_owned();
        let server = spawn(
            &bin("sinkd-srv"),
            &[
                "--bind",
                "127.0.0.1",
                "--sync-root",
                &sync.to_string_lossy(),
                "--state-dir",
                &p("srv-state"),
                "--runtime-dir",
                &p("srv-run"),
                "--zenoh-port",
                &zenoh.to_string(),
                "--data-port",
                &data.to_string(),
                "--pair-port",
                &pair.to_string(),
                "--pair-code",
                code,
            ],
            &log,
        );
        wait_lines(&log, "zenoh tls", 1, Duration::from_secs(20));
        Self { root, sync, log, zenoh, pair, code: code.into(), cli: bin("sinkd"), _server: server }
    }

    fn dir(&self, name: &str) -> PathBuf {
        let dir = self.root.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Pairs a client named `name` with one anchor and its ignore list. Not started yet.
    fn client(&self, name: &str, anchor: &Path, ignore: &[&str]) -> Client {
        let state = self.dir(&format!("{name}-state"));
        let run = self.dir(&format!("{name}-run"));
        let cfg = self.root.path().join(format!("{name}.toml"));
        let list = ignore.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ");
        fs::write(
            &cfg,
            format!(
                "zenoh_port = {}\npair_port = {}\n[[anchors]]\npath = \"{}\"\nignore = [{list}]\n",
                self.zenoh,
                self.pair,
                anchor.to_string_lossy()
            ),
        )
        .unwrap();
        let pair_log = self.root.path().join(format!("{name}-pair.log"));
        let out = fs::File::create(&pair_log).unwrap();
        let status = Command::new(&self.cli)
            .args(["--state-dir", &state.to_string_lossy(), "pair", "127.0.0.1", "--code", &self.code])
            .args(["--pair-port", &self.pair.to_string()])
            .stdout(Stdio::from(out.try_clone().unwrap()))
            .stderr(Stdio::from(out))
            .status()
            .unwrap();
        assert!(status.success(), "pair failed:\n{}", fs::read_to_string(&pair_log).unwrap_or_default());
        Client {
            log: self.root.path().join(format!("{name}.log")),
            args: vec![
                "--config".into(),
                cfg.to_string_lossy().into_owned(),
                "--state-dir".into(),
                state.to_string_lossy().into_owned(),
                "--runtime-dir".into(),
                run.to_string_lossy().into_owned(),
                "--server".into(),
                "127.0.0.1".into(),
            ],
            cli: self.cli.clone(),
            child: None,
        }
    }

    fn logs(&self, clients: &[&Client]) -> String {
        let mut out = format!("server:\n{}", fs::read_to_string(&self.log).unwrap_or_default());
        for client in clients {
            let _ = write!(
                out,
                "\n{}:\n{}",
                client.log.display(),
                fs::read_to_string(&client.log).unwrap_or_default()
            );
        }
        out
    }

    fn wait(&self, what: &str, clients: &[&Client], mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() <= Duration::from_secs(45), "{what}\n{}", self.logs(clients));
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn has(&self, name: &str) -> bool {
        find_file(&self.sync, name).is_some()
    }
}

impl Client {
    fn start(&mut self) {
        let seen = count_lines(&self.log, "watching");
        let args: Vec<&str> = self.args.iter().map(String::as_str).collect();
        self.child = Some(spawn(&self.cli, &args, &self.log));
        wait_lines(&self.log, "watching", seen + 1, Duration::from_secs(20));
    }

    fn stop(&mut self) {
        self.child = None;
    }
}

#[test]
fn client_push_lands_on_server() {
    let hub = Hub::start(17447, "424242");
    let anchor = hub.dir("anchor");
    let mut cli = hub.client("cli", &anchor, &[]);
    cli.start();
    write(&anchor.join("hello.txt"), "hello-from-client\n");
    hub.wait("file did not arrive", &[&cli], || hub.has("hello.txt"));
    let found = find_file(&hub.sync, "hello.txt").unwrap();
    assert_eq!(fs::read_to_string(found).unwrap(), "hello-from-client\n");
}

#[test]
fn ignore_skips_push_and_pull() {
    let hub = Hub::start(17457, "434343");
    let anchor = hub.dir("anchor");
    let mut cli = hub.client("cli", &anchor, &["skip/", "something/", "!something/thing/"]);
    cli.start();
    write(&anchor.join("skip/secret.txt"), "nope\n");
    write(&anchor.join("something/other/hidden.txt"), "no\n");
    write(&anchor.join("something/thing/back.txt"), "back\n");
    write(&anchor.join("keep/visible.txt"), "ok\n");
    hub.wait("sync did not finish", &[&cli], || hub.has("visible.txt") && hub.has("back.txt"));
    assert!(!hub.has("secret.txt"), "ignored dir was sent");
    assert!(!hub.has("hidden.txt"), "ignored sibling of a re-included dir was sent");
}

/// The anchor id is a hash of the path, so two clients share one anchor by taking
/// turns on the same directory.
#[test]
fn pull_delete_keeps_receiver_ignored() {
    let hub = Hub::start(17467, "454545");
    let anchor = hub.dir("anchor");
    let a_dir = hub.root.path().join("anchor-a");
    let mut alice = hub.client("alice", &anchor, &["build/"]);
    alice.start();
    write(&anchor.join("build/local.bin"), "alice-only\n");
    write(&anchor.join("tracked.txt"), "v1\n");
    hub.wait("alice push", &[&alice], || hub.has("tracked.txt"));
    assert!(!hub.has("local.bin"), "ignored build/ was pushed");
    alice.stop();
    fs::rename(&anchor, &a_dir).unwrap();

    fs::create_dir_all(&anchor).unwrap();
    let mut bob = hub.client("bob", &anchor, &["build/"]);
    bob.start();
    // Bob has no basis, so his first push pulls the hub copy first.
    write(&anchor.join("bob.txt"), "b\n");
    hub.wait("bob first pull", &[&bob], || anchor.join("tracked.txt").exists() && hub.has("bob.txt"));
    // Let the first pull and its manifest settle before deleting.
    thread::sleep(Duration::from_secs(1));
    fs::remove_file(anchor.join("tracked.txt")).unwrap();
    hub.wait("bob delete did not reach the store", &[&bob], || !hub.has("tracked.txt"));
    bob.stop();
    fs::remove_dir_all(&anchor).unwrap();
    fs::rename(&a_dir, &anchor).unwrap();

    // Alice is behind. A new local edit forces a push, the hub says Behind, and alice pulls with delete.
    alice.start();
    write(&anchor.join("nudge.txt"), "n\n");
    hub.wait("alice did not pull the delete", &[&alice, &bob], || !anchor.join("tracked.txt").exists());
    assert!(count_lines(&alice.log, "is behind") >= 1, "alice never pulled as behind\n{}", hub.logs(&[&alice]));
    assert_eq!(
        fs::read_to_string(anchor.join("build/local.bin")).unwrap(),
        "alice-only\n",
        "pull with delete removed an ignored local file\n{}",
        hub.logs(&[&alice, &bob])
    );
}

#[test]
fn two_clients_two_rules() {
    let hub = Hub::start(17477, "464646");
    let one = hub.dir("one");
    let two = hub.dir("two");
    let mut c1 = hub.client("c1", &one, &["a_skip/"]);
    let mut c2 = hub.client("c2", &two, &["b_skip/"]);
    c1.start();
    c2.start();
    for i in 0..20 {
        write(&one.join(format!("a_skip/one-hidden-{i}.txt")), "x");
        write(&one.join(format!("b_skip/one-sent-{i}.txt")), "x");
        write(&two.join(format!("b_skip/two-hidden-{i}.txt")), "x");
        write(&two.join(format!("a_skip/two-sent-{i}.txt")), "x");
    }
    hub.wait("both pushes", &[&c1, &c2], || {
        (0..20).all(|i| hub.has(&format!("one-sent-{i}.txt")) && hub.has(&format!("two-sent-{i}.txt")))
    });
    for i in 0..20 {
        assert!(!hub.has(&format!("one-hidden-{i}.txt")), "c1 rules leaked\n{}", hub.logs(&[&c1, &c2]));
        assert!(!hub.has(&format!("two-hidden-{i}.txt")), "c2 rules leaked\n{}", hub.logs(&[&c1, &c2]));
    }
}
