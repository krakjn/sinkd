//! Server and client are separate processes. libreflect's filter hook is process-global,
//! so both sides of one copy cannot run inside a single test process.

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
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.extend(["target", triple, "debug", name]);
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
    let file = fs::File::create(log).unwrap();
    let child = Command::new(bin)
        .args(args)
        .stdout(Stdio::from(file.try_clone().unwrap()))
        .stderr(Stdio::from(file))
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", bin.display()));
    Stop(child)
}

fn wait_line(path: &Path, needle: &str, limit: Duration) {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Ok(file) = fs::File::open(path) {
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if line.contains(needle) {
                    return;
                }
            }
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

#[test]
fn client_push_lands_on_server() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let anchor = root.join("anchor");
    let sync = root.join("sync");
    let server_state = root.join("srv-state");
    let client_state = root.join("cli-state");
    let server_run = root.join("srv-run");
    let client_run = root.join("cli-run");
    fs::create_dir_all(&anchor).unwrap();
    let zenoh = 17447u16;
    let data = 17448u16;
    let pair = 17449u16;
    let client_cfg = root.join("client.toml");
    fs::write(
        &client_cfg,
        format!("zenoh_port = {zenoh}\npair_port = {pair}\n"),
    )
    .unwrap();

    let srv = bin("sinkd-srv");
    let cli = bin("sinkd");
    let srv_log = root.join("srv.log");
    let cli_log = root.join("cli.log");
    let _server = spawn(
        &srv,
        &[
            "--bind",
            "127.0.0.1",
            "--sync-root",
            &sync.to_string_lossy(),
            "--state-dir",
            &server_state.to_string_lossy(),
            "--runtime-dir",
            &server_run.to_string_lossy(),
            "--zenoh-port",
            &zenoh.to_string(),
            "--data-port",
            &data.to_string(),
            "--pair-port",
            &pair.to_string(),
            "--pair-code",
            "424242",
        ],
        &srv_log,
    );
    wait_line(&srv_log, "zenoh tls", Duration::from_secs(20));

    let pair_log = root.join("pair.log");
    let pair_out = fs::File::create(&pair_log).unwrap();
    let status = Command::new(&cli)
        .args([
            "--state-dir",
            &client_state.to_string_lossy(),
            "pair",
            "127.0.0.1",
            "--code",
            "424242",
            "--pair-port",
            &pair.to_string(),
        ])
        .stdout(Stdio::from(pair_out.try_clone().unwrap()))
        .stderr(Stdio::from(pair_out))
        .status()
        .unwrap();
    assert!(status.success(), "pair failed:\n{}", fs::read_to_string(&pair_log).unwrap_or_default());

    let _client = spawn(
        &cli,
        &[
            "--config",
            &client_cfg.to_string_lossy(),
            "--state-dir",
            &client_state.to_string_lossy(),
            "--runtime-dir",
            &client_run.to_string_lossy(),
            "--server",
            "127.0.0.1",
            "--anchor",
            &anchor.to_string_lossy(),
        ],
        &cli_log,
    );
    wait_line(&cli_log, "watching", Duration::from_secs(20));

    fs::write(anchor.join("hello.txt"), "hello-from-client\n").unwrap();
    let start = Instant::now();
    let found = loop {
        if let Some(path) = find_file(&sync, "hello.txt") {
            break path;
        }
        assert!(
            start.elapsed() <= Duration::from_secs(20),
            "file did not arrive\nserver:\n{}\nclient:\n{}",
            fs::read_to_string(&srv_log).unwrap_or_default(),
            fs::read_to_string(&cli_log).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(fs::read_to_string(found).unwrap(), "hello-from-client\n");
}
