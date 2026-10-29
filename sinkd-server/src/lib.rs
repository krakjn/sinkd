//! Hub: Zenoh for control, TCP for file bytes, libreflect on the socket.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sinkd_core::{
    anchor_key, atomic_write, ensure_server_identity, issue_client, load_peers, open_session,
    random_bytes, secrets_eq, serve, socket_path, Role, ServerSettings,
};
use sinkd_proto::{decode, encode, event_key, rpc_key, up_key, Body, HeadEvent, Reply, Request, VERSION};
use sinkd_reflect::{sync_socket, Opts, Side};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Mutex, OwnedMutexGuard};
use tokio::time::timeout;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AnchorStore {
    anchors: HashMap<String, StoredAnchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAnchor {
    generation: u64,
    path: String,
    user: String,
    share: bool,
}

struct Anchor {
    generation: u64,
    path: String,
    user: String,
    share: bool,
    lock: Arc<Mutex<()>>,
}

struct Permit {
    dir: PathBuf,
    client_sends: bool,
    delete: bool,
    update_only: bool,
    anchor_key: String,
    client_id: String,
    guard: OwnedMutexGuard<()>,
}

struct Hub {
    anchors: HashMap<String, Anchor>,
    permits: HashMap<[u8; 16], Permit>,
    peers: HashMap<String, [u8; 32]>,
    sync_root: PathBuf,
}

type SharedHub = Arc<Mutex<Hub>>;

pub async fn run(settings: ServerSettings) -> Result<(), String> {
    std::fs::create_dir_all(&settings.sync_root).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&settings.state_dir).map_err(|e| e.to_string())?;
    let identity = ensure_server_identity(&settings.state_dir)?;
    log::info!("server {}", identity.id);
    let session = open_session(
        &identity.ca_pem,
        &identity.cert_pem,
        &identity.key_pem,
        Role::Server {
            bind: &settings.bind,
            port: settings.zenoh_port,
        },
    )
    .await?;
    let _alive = session
        .liveliness()
        .declare_token(up_key(&identity.id))
        .await
        .map_err(|e| e.to_string())?;

    let hub: SharedHub = Arc::new(Mutex::new(Hub {
        anchors: load_anchors(&settings.sync_root)?,
        permits: HashMap::new(),
        peers: load_peers(&settings.state_dir)?,
        sync_root: settings.sync_root.clone(),
    }));

    let (stop_tx, mut stop_rx) = watch::channel(false);
    let (reload_tx, mut reload_rx) = tokio::sync::mpsc::unbounded_channel();
    {
        let stop_tx = stop_tx.clone();
        let path = socket_path(&settings.runtime_dir, "sinkd-srv.sock");
        tokio::spawn(async move {
            if let Err(e) = serve(path, stop_tx.clone(), stop_tx.subscribe(), reload_tx).await {
                log::warn!("control socket: {e}");
            }
        });
    }

    let pair_listener = TcpListener::bind(format!("{}:{}", settings.bind, settings.pair_port))
        .await
        .map_err(|e| format!("pair port: {e}"))?;
    let data_listener = TcpListener::bind(format!("{}:{}", settings.bind, settings.data_port))
        .await
        .map_err(|e| format!("data port: {e}"))?;
    log::info!(
        "zenoh tls/{}:{}  pair {}  data {}",
        settings.bind,
        settings.zenoh_port,
        settings.pair_port,
        settings.data_port
    );
    let rpc = rpc_key(&identity.id);
    let queryable = session.declare_queryable(&rpc).await.map_err(|e| e.to_string())?;

    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() { break; }
            }
            _ = tokio::signal::ctrl_c() => { let _ = stop_tx.send(true); }
            _ = reload_rx.recv() => {
                if let Ok(peers) = load_peers(&settings.state_dir) {
                    hub.lock().await.peers = peers;
                    log::info!("reloaded peers");
                }
            }
            accepted = pair_listener.accept() => {
                if let Ok((sock, addr)) = accepted {
                    let state = settings.state_dir.clone();
                    let code = settings.fixed_pair_code.clone();
                    let hub = Arc::clone(&hub);
                    tokio::spawn(async move {
                        if let Err(e) = handle_pair(sock, &state, code.as_deref(), &hub).await {
                            log::warn!("pair {addr}: {e}");
                        }
                    });
                }
            }
            accepted = data_listener.accept() => {
                if let Ok((sock, addr)) = accepted {
                    let hub = Arc::clone(&hub);
                    let session = session.clone();
                    let server_id = identity.id.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_data(sock, &hub, &session, &server_id).await {
                            log::warn!("data {addr}: {e}");
                        }
                    });
                }
            }
            incoming = queryable.recv_async() => {
                let Ok(query) = incoming else { continue; };
                let hub = Arc::clone(&hub);
                let rpc = rpc.clone();
                let data_port = settings.data_port;
                tokio::spawn(async move {
                    let reply = answer(&hub, data_port, query.payload()).await;
                    if let Ok(bytes) = encode(&reply) {
                        let _ = query.reply(&rpc, bytes).await;
                    }
                });
            }
        }
    }
    let _ = session.close().await;
    Ok(())
}

async fn handle_pair(mut sock: TcpStream, state: &Path, fixed: Option<&str>, hub: &SharedHub) -> Result<(), String> {
    let client_id = String::from_utf8(read_frame(&mut sock).await?).map_err(|e| e.to_string())?;
    let client_id = client_id.trim().to_string();
    let code = match fixed {
        Some(code) => code.to_string(),
        None => format!("{:06}", u32::from_be_bytes(random_bytes::<4>()?) % 1_000_000),
    };
    eprintln!("pair code {code} for client {client_id}");
    write_frame(&mut sock, b"ready").await?;
    let typed = String::from_utf8(read_frame(&mut sock).await?).map_err(|e| e.to_string())?;
    if typed.trim() != code {
        write_frame(&mut sock, b"no").await?;
        return Err("pair code rejected".into());
    }
    let server_id = std::fs::read_to_string(state.join("id")).map_err(|e| e.to_string())?;
    let (ca, cert, key, secret) = issue_client(state, &client_id)?;
    hub.lock().await.peers = load_peers(state)?;
    write_frame(&mut sock, b"ok").await?;
    write_frame(&mut sock, server_id.trim().as_bytes()).await?;
    write_frame(&mut sock, ca.as_bytes()).await?;
    write_frame(&mut sock, cert.as_bytes()).await?;
    write_frame(&mut sock, key.as_bytes()).await?;
    write_frame(&mut sock, &secret).await?;
    log::info!("paired {client_id}");
    Ok(())
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

async fn answer(hub: &SharedHub, data_port: u16, payload: Option<&zenoh::bytes::ZBytes>) -> Reply {
    let Some(payload) = payload else {
        return Reply::Failed { message: "empty query".into() };
    };
    let request: Request = match decode(&payload.to_bytes()) {
        Ok(request) => request,
        Err(e) => return Reply::Failed { message: e },
    };
    if request.version != VERSION {
        return Reply::Failed { message: "protocol version mismatch".into() };
    }
    match &request.body {
        Body::Push { anchor_id, path, user, share, basis } => {
            grant(hub, data_port, &request, anchor_id, path, user, *share, Some(*basis), true, true, false).await
        }
        Body::Pull { anchor_id, path, user, share, delete, update_only } => {
            grant(hub, data_port, &request, anchor_id, path, user, *share, None, false, *delete, *update_only).await
        }
    }
}

#[allow(clippy::fn_params_excessive_bools)]
async fn grant(
    hub: &SharedHub,
    data_port: u16,
    request: &Request,
    anchor_id: &str,
    path: &str,
    user: &str,
    share: bool,
    basis: Option<u64>,
    client_sends: bool,
    delete: bool,
    update_only: bool,
) -> Reply {
    let key = anchor_key(user, anchor_id, share);
    if let Err(e) = ensure_anchor(hub, &key, path, user, share).await {
        return Reply::Failed { message: e };
    }
    let lock = {
        let hub = hub.lock().await;
        match auth(&hub, request) {
            Ok(()) => {}
            Err(message) => return Reply::Failed { message },
        }
        let Some(anchor) = hub.anchors.get(&key) else {
            return Reply::Failed { message: "missing anchor".into() };
        };
        if let Some(basis) = basis {
            if basis != anchor.generation {
                return Reply::Behind { head: anchor.generation };
            }
        }
        Arc::clone(&anchor.lock)
    };
    let guard = lock.lock_owned().await;
    let mut locked = hub.lock().await;
    let Some(anchor) = locked.anchors.get(&key) else {
        return Reply::Failed { message: "missing anchor".into() };
    };
    if let Some(basis) = basis {
        if basis != anchor.generation {
            return Reply::Behind { head: anchor.generation };
        }
    }
    let head = anchor.generation;
    let dir = locked.sync_root.join(&key);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Reply::Failed { message: e.to_string() };
    }
    let Ok(token) = random_bytes::<16>() else {
        return Reply::Failed { message: "token entropy failed".into() };
    };
    locked.permits.insert(
        token,
        Permit {
            dir,
            client_sends,
            delete,
            update_only,
            anchor_key: key,
            client_id: request.client_id.clone(),
            guard,
        },
    );
    drop(locked);
    let hub = Arc::clone(hub);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        if hub.lock().await.permits.remove(&token).is_some() {
            log::info!("grant expired");
        }
    });
    Reply::Granted {
        data_port,
        token,
        head,
        client_sends,
        delete,
        update_only,
    }
}

fn auth(hub: &Hub, request: &Request) -> Result<(), String> {
    let Some(secret) = hub.peers.get(&request.client_id) else {
        return Err("client is not paired".into());
    };
    if secrets_eq(secret, &request.secret) {
        Ok(())
    } else {
        Err("bad client secret".into())
    }
}

async fn ensure_anchor(hub: &SharedHub, key: &str, path: &str, user: &str, share: bool) -> Result<(), String> {
    let mut locked = hub.lock().await;
    locked.anchors.entry(key.to_string()).or_insert_with(|| Anchor {
        generation: 0,
        path: path.to_string(),
        user: user.to_string(),
        share,
        lock: Arc::new(Mutex::new(())),
    });
    let sync_root = locked.sync_root.clone();
    save_anchors(&locked, &sync_root)
}

async fn handle_data(sock: TcpStream, hub: &SharedHub, session: &zenoh::Session, server_id: &str) -> Result<(), String> {
    let mut std_sock = sock.into_std().map_err(|e| e.to_string())?;
    std_sock.set_nonblocking(false).map_err(|e| e.to_string())?;
    let _ = std_sock.set_nodelay(true);
    let _ = std_sock.set_read_timeout(Some(Duration::from_secs(120)));
    let _ = std_sock.set_write_timeout(Some(Duration::from_secs(120)));
    let mut token = [0u8; 16];
    std::io::Read::read_exact(&mut std_sock, &mut token).map_err(|e| format!("token: {e}"))?;
    let permit = hub
        .lock()
        .await
        .permits
        .remove(&token)
        .ok_or_else(|| "unknown or expired token".to_string())?;
    let Permit { dir, client_sends, delete, update_only, anchor_key, client_id, guard } = permit;
    let side = if client_sends { Side::Receive } else { Side::Send };
    let opts = Opts { delete, update_only };
    let stats = tokio::task::spawn_blocking(move || sync_socket(&std_sock, side, &dir, opts))
        .await
        .map_err(|e| e.to_string())??;
    log::info!("{anchor_key}: files={} bytes={}", stats.files, stats.bytes);
    if client_sends {
        let (head, sync_root) = {
            let mut locked = hub.lock().await;
            let anchor = locked.anchors.get_mut(&anchor_key).ok_or("missing anchor")?;
            anchor.generation = anchor.generation.saturating_add(1);
            (anchor.generation, locked.sync_root.clone())
        };
        save_anchors(&*hub.lock().await, &sync_root)?;
        let event = HeadEvent { anchor_key: anchor_key.clone(), head, writer: client_id };
        session
            .put(event_key(server_id, &anchor_key), encode(&event)?)
            .await
            .map_err(|e| e.to_string())?;
    }
    drop(guard);
    Ok(())
}

fn anchors_file(sync_root: &Path) -> PathBuf {
    sync_root.join(".sinkd").join("anchors.toml")
}

fn load_anchors(sync_root: &Path) -> Result<HashMap<String, Anchor>, String> {
    let path = anchors_file(sync_root);
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let store: AnchorStore = toml::from_str(&text).map_err(|e| e.to_string())?;
    Ok(store
        .anchors
        .into_iter()
        .map(|(key, stored)| {
            (
                key,
                Anchor {
                    generation: stored.generation,
                    path: stored.path,
                    user: stored.user,
                    share: stored.share,
                    lock: Arc::new(Mutex::new(())),
                },
            )
        })
        .collect())
}

fn save_anchors(hub: &Hub, sync_root: &Path) -> Result<(), String> {
    let store = AnchorStore {
        anchors: hub
            .anchors
            .iter()
            .map(|(key, anchor)| {
                (
                    key.clone(),
                    StoredAnchor {
                        generation: anchor.generation,
                        path: anchor.path.clone(),
                        user: anchor.user.clone(),
                        share: anchor.share,
                    },
                )
            })
            .collect(),
    };
    let text = toml::to_string_pretty(&store).map_err(|e| e.to_string())?;
    atomic_write(&anchors_file(sync_root), text.as_bytes())
}
