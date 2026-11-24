//! Watches anchors, asks the server for a token, and runs libreflect on a TCP socket.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use sinkd_core::{
    anchor_id, anchor_key, ensure_client_id, hostname, load_client_bundle, manifest_path, open_session,
    preserve, read_basis, save_client_bundle, serve, socket_path, user_name, write_basis, Role,
    Watch,
};
use sinkd_core::ClientSettings;
use sinkd_proto::{decode, encode, events_selector, rpc_key, Body, HeadEvent, Reply, Request, VERSION};
use sinkd_reflect::{sync_socket, Opts, Side};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::timeout;

pub use sinkd_core::{load_client, ClientOverrides};

pub async fn pair(state: &Path, host: &str, port: u16, code: Option<String>) -> Result<(), String> {
    let client_id = ensure_client_id(state)?;
    let mut sock = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("pair {host}:{port}: {e}"))?;
    write_frame(&mut sock, client_id.as_bytes()).await?;
    let ready = read_frame(&mut sock).await?;
    if ready != b"ready" {
        return Err("server did not accept a pair".into());
    }
    let code = if let Some(code) = code {
        code
    } else {
        eprint!("code from the server: ");
        let line = tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
            Ok::<String, String>(line)
        })
        .await
        .map_err(|e| e.to_string())??;
        line.trim().to_string()
    };
    write_frame(&mut sock, code.trim().as_bytes()).await?;
    let status = read_frame(&mut sock).await?;
    if status != b"ok" {
        return Err("pair rejected".into());
    }
    let server_id = String::from_utf8(read_frame(&mut sock).await?).map_err(|e| e.to_string())?;
    let ca = String::from_utf8(read_frame(&mut sock).await?).map_err(|e| e.to_string())?;
    let cert = String::from_utf8(read_frame(&mut sock).await?).map_err(|e| e.to_string())?;
    let key = String::from_utf8(read_frame(&mut sock).await?).map_err(|e| e.to_string())?;
    let secret_bytes = read_frame(&mut sock).await?;
    let secret: [u8; 32] = secret_bytes
        .try_into()
        .map_err(|_| "server secret was not 32 bytes".to_string())?;
    save_client_bundle(state, server_id.trim(), host, &ca, &cert, &key, &secret)?;
    log::info!("paired with {host} ({})", server_id.trim());
    Ok(())
}

pub async fn run(settings: ClientSettings) -> Result<(), String> {
    let bundle = load_client_bundle(&settings.state_dir)?;
    let session = open_session(
        &bundle.ca_pem,
        &bundle.cert_pem,
        &bundle.key_pem,
        Role::Client {
            host: &settings.server,
            port: settings.zenoh_port,
        },
    )
    .await?;
    let rpc = rpc_key(&bundle.server_id);
    let subscriber = session
        .declare_subscriber(events_selector(&bundle.server_id))
        .await
        .map_err(|e| e.to_string())?;

    let (stop_tx, mut stop_rx) = watch::channel(false);
    let (reload_tx, mut reload_rx) = tokio::sync::mpsc::unbounded_channel();
    {
        let stop_tx = stop_tx.clone();
        let path = socket_path(&settings.runtime_dir, "sinkd.sock");
        tokio::spawn(async move {
            if let Err(e) = serve(path, stop_tx.clone(), stop_tx.subscribe(), reload_tx).await {
                log::warn!("control socket: {e}");
            }
        });
    }

    let applying = Arc::new(AtomicBool::new(false));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();
    let flag = Arc::clone(&applying);
    let mut debouncer = new_debouncer(
        Duration::from_millis(400),
        None,
        move |result: notify_debouncer_full::DebounceEventResult| {
            if flag.load(Ordering::Relaxed) {
                return;
            }
            let Ok(events) = result else { return };
            for event in events {
                for path in event.event.paths {
                    let _ = tx.send(path);
                }
            }
        },
    )
    .map_err(|e| e.to_string())?;

    let watches = prepare(&settings.watches)?;
    for watch in &watches {
        debouncer
            .watch(&watch.path, RecursiveMode::Recursive)
            .map_err(|e| format!("{}: {e}", watch.path.display()))?;
        log::info!("watching {}", watch.path.display());
    }
    let mut rescan = tokio::time::interval(Duration::from_secs(30));
    rescan.tick().await;

    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() { break; }
            }
            _ = tokio::signal::ctrl_c() => { let _ = stop_tx.send(true); }
            _ = reload_rx.recv() => {
                log::info!("reload is not wired to a new config yet; restart to pick up anchors");
            }
            Some(path) = rx.recv() => {
                if let Some(watch) = watch_for(&watches, &path) {
                    let watch = watch.clone();
                    if ignored_event(&watch, &path) {
                        continue;
                    }
                    // force is false so a pull's own writes, once the manifest matches, do not push again.
                    if let Err(e) = sync_anchor(&session, &rpc, &settings, &bundle.id, &bundle.secret, &watch, false, &applying).await {
                        log::warn!("{}: {e}", watch.path.display());
                    }
                }
            }
            Ok(sample) = subscriber.recv_async() => {
                let event: HeadEvent = match decode(&sample.payload().to_bytes()) {
                    Ok(event) => event,
                    Err(e) => {
                        log::warn!("event: {e}");
                        continue;
                    }
                };
                if event.writer == bundle.id {
                    continue;
                }
                if let Some(watch) = watches.iter().find(|w| {
                    anchor_key(&user_name(), &anchor_id(&w.path.to_string_lossy()), w.share) == event.anchor_key
                }) {
                    let watch = watch.clone();
                    let manifest = sinkd_core::manifest::load(&manifest_path(&settings.state_dir, &key_of(&watch)));
                    let result = if sinkd_core::dirty_with(
                        &watch.path,
                        &manifest,
                        &sinkd_core::Rules::load(&watch.path, &watch.ignore),
                    )
                    .is_empty()
                    {
                        pull(&session, &rpc, &settings, &bundle.id, &bundle.secret, &watch, true, false, &applying).await
                    } else {
                        sync_anchor(&session, &rpc, &settings, &bundle.id, &bundle.secret, &watch, false, &applying).await
                    };
                    if let Err(e) = result {
                        log::warn!("pull {}: {e}", watch.path.display());
                    }
                }
            }
            _ = rescan.tick() => {
                for watch in &watches.clone() {
                    let manifest = sinkd_core::manifest::load(&manifest_path(&settings.state_dir, &key_of(watch)));
                    if sinkd_core::dirty(&watch.path, &manifest).is_empty() {
                        continue;
                    }
                    if let Err(e) = sync_anchor(&session, &rpc, &settings, &bundle.id, &bundle.secret, watch, false, &applying).await {
                        log::warn!("rescan {}: {e}", watch.path.display());
                    }
                }
            }
        }
    }
    let _ = watches;
    let _ = session.close().await;
    Ok(())
}

fn prepare(watches: &[Watch]) -> Result<Vec<Watch>, String> {
    let mut out = Vec::new();
    for watch in watches {
        std::fs::create_dir_all(&watch.path).map_err(|e| format!("{}: {e}", watch.path.display()))?;
        let path = watch.path.canonicalize().map_err(|e| format!("{}: {e}", watch.path.display()))?;
        out.push(Watch { path, share: watch.share, ignore: watch.ignore.clone() });
    }
    Ok(out)
}

fn ignored_event(watch: &Watch, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(&watch.path) else { return false };
    let rel = rel.to_string_lossy().replace('\\', "/");
    if rel.is_empty() {
        return false;
    }
    let is_dir = path.is_dir();
    sinkd_core::Rules::load(&watch.path, &watch.ignore).ignored(&rel, is_dir)
}

fn watch_for(watches: &[Watch], path: &Path) -> Option<Watch> {
    watches
        .iter()
        .filter(|watch| path.starts_with(&watch.path))
        .max_by_key(|watch| watch.path.as_os_str().len())
        .cloned()
}

fn key_of(watch: &Watch) -> String {
    anchor_key(&user_name(), &anchor_id(&watch.path.to_string_lossy()), watch.share)
}

async fn sync_anchor(
    session: &zenoh::Session,
    rpc: &str,
    settings: &ClientSettings,
    client_id: &str,
    secret: &[u8; 32],
    watch: &Watch,
    force: bool,
    applying: &AtomicBool,
) -> Result<(), String> {
    let key = key_of(watch);
    let manifest_file = manifest_path(&settings.state_dir, &key);
    let rules = sinkd_core::Rules::load(&watch.path, &watch.ignore);
    let was_dirty = !sinkd_core::dirty_with(
        &watch.path,
        &sinkd_core::manifest::load(&manifest_file),
        &rules,
    )
    .is_empty();
    if read_basis(&settings.state_dir, &key).is_none() {
        pull(session, rpc, settings, client_id, secret, watch, false, true, applying).await?;
    }
    let still_dirty = !sinkd_core::dirty_with(
        &watch.path,
        &sinkd_core::manifest::load(&manifest_file),
        &rules,
    )
    .is_empty();
    if !force && !was_dirty && !still_dirty {
        return Ok(());
    }
    let basis = read_basis(&settings.state_dir, &key).unwrap_or(0);
    let request = Request {
        version: VERSION,
        client_id: client_id.to_string(),
        secret: *secret,
        body: Body::Push {
            anchor_id: anchor_id(&watch.path.to_string_lossy()),
            path: watch.path.display().to_string(),
            user: user_name(),
            share: watch.share,
            basis,
        },
    };
    match rpc_call(session, rpc, &request).await? {
        Reply::Granted { data_port, token, head, client_sends, delete, update_only } => {
            transfer(settings, data_port, token, watch, client_sends, delete, update_only, applying).await?;
            write_basis(&settings.state_dir, &key, head.saturating_add(1))?;
            let scanned = sinkd_core::scan_with(&watch.path, &sinkd_core::Rules::load(&watch.path, &watch.ignore));
            sinkd_core::manifest::store(&manifest_path(&settings.state_dir, &key), &scanned)?;
            Ok(())
        }
        Reply::Behind { head } => {
            log::info!("{} is behind {head}; keeping local edits aside", watch.path.display());
            let manifest = sinkd_core::manifest::load(&manifest_path(&settings.state_dir, &key));
            let dirty = sinkd_core::dirty_with(
                &watch.path,
                &manifest,
                &sinkd_core::Rules::load(&watch.path, &watch.ignore),
            );
            let saved = preserve(
                &watch.path,
                &dirty,
                &hostname(),
                &settings.state_dir.join("conflicts").join(&key),
            )?;
            for path in &saved {
                log::warn!("kept {}", path.display());
            }
            pull(session, rpc, settings, client_id, secret, watch, true, false, applying).await?;
            Ok(())
        }
        Reply::Failed { message } => Err(message),
    }
}

async fn pull(
    session: &zenoh::Session,
    rpc: &str,
    settings: &ClientSettings,
    client_id: &str,
    secret: &[u8; 32],
    watch: &Watch,
    delete: bool,
    update_only: bool,
    applying: &AtomicBool,
) -> Result<(), String> {
    let request = Request {
        version: VERSION,
        client_id: client_id.to_string(),
        secret: *secret,
        body: Body::Pull {
            anchor_id: anchor_id(&watch.path.to_string_lossy()),
            path: watch.path.display().to_string(),
            user: user_name(),
            share: watch.share,
            delete,
            update_only,
        },
    };
    match rpc_call(session, rpc, &request).await? {
        Reply::Granted { data_port, token, head, client_sends, delete, update_only } => {
            transfer(settings, data_port, token, watch, client_sends, delete, update_only, applying).await?;
            let key = key_of(watch);
            write_basis(&settings.state_dir, &key, head)?;
            let scanned = sinkd_core::scan_with(&watch.path, &sinkd_core::Rules::load(&watch.path, &watch.ignore));
            sinkd_core::manifest::store(&manifest_path(&settings.state_dir, &key), &scanned)?;
            Ok(())
        }
        Reply::Behind { head } => Err(format!("unexpected behind {head} on pull")),
        Reply::Failed { message } => Err(message),
    }
}

async fn transfer(
    settings: &ClientSettings,
    data_port: u16,
    token: [u8; 16],
    watch: &Watch,
    client_sends: bool,
    delete: bool,
    update_only: bool,
    applying: &AtomicBool,
) -> Result<(), String> {
    let host = settings.server.clone();
    let dir = watch.path.clone();
    let rules = sinkd_core::Rules::load(&watch.path, &watch.ignore);
    applying.store(true, Ordering::Relaxed);
    let result = tokio::task::spawn_blocking(move || {
        let sock = std::net::TcpStream::connect((host.as_str(), data_port)).map_err(|e| e.to_string())?;
        let _ = sock.set_nodelay(true);
        let _ = sock.set_read_timeout(Some(Duration::from_secs(120)));
        let _ = sock.set_write_timeout(Some(Duration::from_secs(120)));
        std::io::Write::write_all(&mut &sock, &token).map_err(|e| e.to_string())?;
        let side = if client_sends { Side::Send } else { Side::Receive };
        let _guard = JobIgnore::arm(dir.clone(), rules);
        sync_socket(&sock, side, &dir, Opts { delete, update_only }).map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?;
    applying.store(false, Ordering::Relaxed);
    result
}

struct JobIgnore;
impl JobIgnore {
    fn arm(root: PathBuf, rules: sinkd_core::Rules) -> Self {
        *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Active { root, rules });
        sinkd_reflect::set_extra_filter(Some(extra_skip));
        Self
    }
}
impl Drop for JobIgnore {
    fn drop(&mut self) {
        *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = None;
        sinkd_reflect::set_extra_filter(None);
    }
}

struct Active {
    root: PathBuf,
    rules: sinkd_core::Rules,
}

static ACTIVE: std::sync::Mutex<Option<Active>> = std::sync::Mutex::new(None);

fn extra_skip(path: &str, is_dir: bool) -> bool {
    let guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(active) = guard.as_ref() else { return false };
    let rel = path.strip_prefix(&*active.root.to_string_lossy()).unwrap_or(path);
    let rel = rel.trim_start_matches(['/', '\\']).replace('\\', "/");
    active.rules.ignored(&rel, is_dir)
}

async fn rpc_call(session: &zenoh::Session, key: &str, request: &Request) -> Result<Reply, String> {
    let replies = session
        .get(key)
        .payload(encode(request)?)
        .timeout(Duration::from_secs(60))
        .await
        .map_err(|e| e.to_string())?;
    let reply = replies.recv_async().await.map_err(|e| e.to_string())?;
    let sample = reply.result().map_err(std::string::ToString::to_string)?;
    decode(&sample.payload().to_bytes())
}

async fn read_frame(sock: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    timeout(Duration::from_secs(60), sock.read_exact(&mut len_buf))
        .await
        .map_err(|_| "pair timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let len = usize::try_from(u32::from_be_bytes(len_buf)).map_err(|_| "bad frame".to_string())?;
    if len > 1_000_000 {
        return Err("pair frame too large".into());
    }
    let mut buf = vec![0u8; len];
    timeout(Duration::from_secs(60), sock.read_exact(&mut buf))
        .await
        .map_err(|_| "pair timed out".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

async fn write_frame(sock: &mut TcpStream, bytes: &[u8]) -> Result<(), String> {
    let len = u32::try_from(bytes.len()).map_err(|_| "frame too large".to_string())?;
    sock.write_all(&len.to_be_bytes()).await.map_err(|e| e.to_string())?;
    sock.write_all(bytes).await.map_err(|e| e.to_string())?;
    Ok(())
}

pub fn stop_running(runtime: &Path) -> Result<(), String> {
    sinkd_core::notify(&socket_path(runtime, "sinkd.sock"), "stop")
}

pub fn reload_running(runtime: &Path) -> Result<(), String> {
    sinkd_core::notify(&socket_path(runtime, "sinkd.sock"), "reload")
}
