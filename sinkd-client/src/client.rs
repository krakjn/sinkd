use clap::parser::ValuesRef;
use log::{debug, error, info, warn};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc, Mutex, RwLock,
    },
    thread,
    time::{Duration, Instant},
};

use crate::sync_state::{
    attach_client_outbound_basis, client_state_dir, load_client_sync_state, mark_local_dirty,
    maybe_record_writer_ack, record_pull_acked, ClientSyncState,
};
use crate::{daemon, parameters::ClientParameters};
use sinkd_core::{
    bad,
    conflict,
    config::RsyncOptions,
    ipc,
    outcome::Outcome,
    rsync::{
        check_behind_source, rsync_behind_mirror_sync, skip_missing_behind_source,
    },
};
use crate::config;
use crate::data::{Anchor, InodeMap};

/// Linux/Android use inotify-style backends without extra polling; other platforms fall back to periodic poll.
fn notify_config_for_platform() -> notify::Config {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        notify::Config::default()
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        notify::Config::default().with_poll_interval(Duration::from_secs(1))
    }
}

pub fn start(params: &ClientParameters) -> Outcome<()> {
    println!("logging to: {}", params.shared.log_path.display());
    daemon::spawn(params)
}

pub fn stop(_params: &ClientParameters) -> Outcome<()> {
    ipc::send_terminate_signal()
}

pub fn restart(params: &ClientParameters) -> Outcome<()> {
    match stop(params) {
        Ok(()) => {
            start(params)?;
            Ok(())
        }
        Err(e) => bad!(e),
    }
}

fn notify_reload() {
    if let Err(e) = ipc::publish_config_reload_signal() {
        warn!("config updated but could not publish reload notification over Zenoh: {e}");
    }
}

/// Anchors / users / list / log — same entrypoints as `sinkd client …` (daemon: [`start`], [`stop`], [`restart`]).
pub fn add(
    params: &ClientParameters,
    share_paths: &[&String],
    user_paths: &[&String],
) -> Outcome<()> {
    if share_paths.is_empty() && user_paths.is_empty() {
        return bad!("add: supply at least one --share and/or PATH");
    }

    if !share_paths.is_empty() {
        let sys_cfg_path = config::get_system_config_path(params)?;
        let mut sys_cfg = config::load_system_config(&sys_cfg_path)?;
        let anchors = sys_cfg.anchors.get_or_insert_with(Vec::new);
        for p in share_paths {
            let resolved = sinkd_core::resolve(p)?;
            if anchors.iter().any(|a| a.path == resolved) {
                continue;
            }
            anchors.push(Anchor::from(resolved));
        }
        config::save_system_config(&sys_cfg_path, &sys_cfg)?;
        info!("updated system config {}", sys_cfg_path.display());
    }

    if !user_paths.is_empty() {
        let user_cfg_paths = config::get_user_config_paths(params)?;
        for user_cfg_path in user_cfg_paths {
            let mut usr_cfg = config::load_user_config(&user_cfg_path)?;
            for p in user_paths {
                let resolved = sinkd_core::resolve(p)?;
                if usr_cfg.anchors.iter().any(|a| a.path == resolved) {
                    continue;
                }
                usr_cfg.anchors.push(Anchor::from(resolved));
            }
            config::save_user_config(&user_cfg_path, &usr_cfg)?;
            info!("updated user config {}", user_cfg_path.display());
        }
    }

    notify_reload();
    Ok(())
}

pub fn rm(
    params: &ClientParameters,
    share_paths: &[&String],
    user_paths: &[&String],
) -> Outcome<()> {
    if share_paths.is_empty() && user_paths.is_empty() {
        return bad!("remove: supply at least one --share and/or PATH");
    }

    let sys_path = config::get_system_config_path(params)?;

    if !share_paths.is_empty() {
        let mut sys = config::load_system_config(&sys_path)?;
        if let Some(anchors) = sys.anchors.as_mut() {
            for p in share_paths {
                let resolved = sinkd_core::resolve(p)?;
                anchors.retain(|a| a.path != resolved);
            }
        }
        config::save_system_config(&sys_path, &sys)?;
        info!("updated system config {}", sys_path.display());
    }

    if !user_paths.is_empty() {
        let user_cfg_paths = config::get_user_config_paths(params)?;
        for user_path in user_cfg_paths {
            let mut usr = config::load_user_config(&user_path)?;
            for p in user_paths {
                let resolved = sinkd_core::resolve(p)?;
                usr.anchors.retain(|a| a.path != resolved);
            }
            config::save_user_config(&user_path, &usr)?;
            info!("updated user config {}", user_path.display());
        }
    }

    notify_reload();
    Ok(())
}

pub fn adduser(params: &ClientParameters, users: Option<ValuesRef<String>>) -> Outcome<()> {
    let Some(users) = users else {
        return bad!("no user(s) were given!");
    };
    let sys_path = config::get_system_config_path(params)?;
    let mut sys = config::load_system_config(&sys_path)?;
    for user in users {
        if !sys.users.iter().any(|u| u == user.as_str()) {
            sys.users.push(user.as_str().to_string());
        }
    }
    config::save_system_config(&sys_path, &sys)?;
    info!("updated system config {}", sys_path.display());
    notify_reload();
    Ok(())
}

pub fn rmuser(params: &ClientParameters, users: Option<ValuesRef<String>>) -> Outcome<()> {
    let Some(users) = users else {
        return bad!("no user(s) were given!");
    };
    let sys_path = config::get_system_config_path(params)?;
    let mut sys = config::load_system_config(&sys_path)?;
    for user in users {
        sys.users.retain(|u| u != user.as_str());
    }
    config::save_system_config(&sys_path, &sys)?;
    info!("updated system config {}", sys_path.display());
    notify_reload();
    Ok(())
}

pub fn ls(params: &ClientParameters, paths: Option<Vec<&String>>) -> Outcome<()> {
    let inode_map = config::load_inode_map(params)?;
    let mut keys: Vec<_> = inode_map.keys().cloned().collect();
    keys.sort();

    if let Some(filter) = paths {
        if filter.is_empty() {
            return bad!("no paths were given!");
        }
        let resolved: Vec<PathBuf> = filter
            .iter()
            .map(|p| sinkd_core::resolve(p))
            .collect::<Result<_, _>>()?;
        for k in keys {
            if resolved.iter().any(|root| k.starts_with(root)) {
                println!("{}", k.display());
            }
        }
    } else {
        for k in keys {
            println!("{}", k.display());
        }
    }
    Ok(())
}

pub fn log(params: &ClientParameters) -> Outcome<()> {
    let data = fs::read_to_string(&params.shared.log_path).map_err(|e| {
        format!(
            "couldn't read log file {}: {e}",
            params.shared.log_path.display()
        )
    })?;
    print!("{data}");
    Ok(())
}

// Daemonized call, stdin/stdout/stderr are closed
pub fn run(params: &ClientParameters) -> Outcome<()> {
    let client_sync = load_client_sync_state(params)?;
    let params = Arc::new(params.clone());
    let inode_map = config::load_inode_map(&params)?;

    let (notify_tx, notify_rx): (mpsc::Sender<notify::Event>, mpsc::Receiver<notify::Event>) =
        mpsc::channel();
    let (event_tx, event_rx): (mpsc::Sender<PathBuf>, mpsc::Receiver<PathBuf>) = mpsc::channel();

    let watchers: Arc<Mutex<Vec<RecommendedWatcher>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let initial = setup_watchers(&inode_map, notify_tx.clone())?;
        *watchers
            .lock()
            .map_err(|e| format!("watchers lock poisoned: {e}"))? = initial;
    }

    let fatal = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&fatal))?;

    let inodes = Arc::new(RwLock::new(inode_map));
    let local_dirty = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));

    let watch_thread = thread::spawn({
        let fatal = Arc::clone(&fatal);
        let inode_map = Arc::clone(&inodes);
        let local_dirty = Arc::clone(&local_dirty);
        // watch_thread needs a mutable map to assign "last event" to inode
        move || watch_entry(inode_map, notify_rx, event_tx, fatal, local_dirty)
    });

    let zenoh_thread = thread::spawn({
        let fatal = Arc::clone(&fatal);
        let inode_map = Arc::clone(&inodes);
        let params = Arc::clone(&params);
        let watchers = Arc::clone(&watchers);
        let notify_tx = notify_tx.clone();
        let client_sync = Arc::clone(&client_sync);
        let local_dirty = Arc::clone(&local_dirty);
        move || {
            zenoh_entry(
                inode_map,
                event_rx,
                fatal,
                params,
                watchers,
                notify_tx,
                client_sync,
                local_dirty,
            )
        }
    });

    match watch_thread.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => error!("{e}"),
        Err(join_err) => return bad!("client:watch_thread join error! >> {:?}", join_err),
    }
    match zenoh_thread.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => error!("{e}"),
        Err(join_err) => return bad!("client:zenoh_thread join error! >> {:?}", join_err),
    }
    Ok(())
}

// This will check the event path against the known paths passed at config time
// Only top level paths are sent to the synch thread if the watched directory has exceeded
// interval. In other words events are filtered against intervals (per inode) and added
// to the synch queue.
fn check_interval(
    event_path: &Path,
    inode_map: &Arc<RwLock<InodeMap>>,
    event_tx: &mpsc::Sender<PathBuf>,
) -> Outcome<()> {
    // need to dynamically lookup keys and compare path names
    debug!("checking interval, event:{}", event_path.display());
    if let Ok(mut inode_map) = inode_map.write() {
        for (inode_path, inode) in inode_map.iter_mut() {
            if event_path.starts_with(inode_path) {
                let now = Instant::now();
                let elapse = now.duration_since(inode.last_event);
                if elapse >= inode.interval {
                    debug!("EVENT>> elapse: {}", elapse.as_secs());
                    inode.last_event = now;
                    if let Err(e) = event_tx.send(inode_path.clone()) {
                        return bad!("unable to send event path to sync queue: {}", e);
                    }
                }
                break;
            }
        }
        Ok(())
    } else {
        bad!("Unable to acquire RwLock for inode_map")
    }
}

fn watch_entry(
    inode_map: Arc<RwLock<InodeMap>>,
    notify_rx: mpsc::Receiver<notify::Event>,
    event_tx: mpsc::Sender<PathBuf>,
    fatal: Arc<AtomicBool>,
    local_dirty: Arc<Mutex<HashSet<PathBuf>>>,
) -> Outcome<()> {
    loop {
        if fatal.load(Ordering::Relaxed) {
            info!("client:watch_entry>> aborting");
            return Ok(());
        }

        match notify_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(event) => {
                if matches!(
                    event.kind,
                    notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_)
                ) {
                    let track_dirty = matches!(
                        event.kind,
                        notify::EventKind::Create(_) | notify::EventKind::Modify(_)
                    );
                    for path in &event.paths {
                        if track_dirty {
                            mark_local_dirty(local_dirty.as_ref(), path);
                        }
                        check_interval(path, &inode_map, &event_tx)?;
                    }
                }
            }
            Err(err) => match err {
                RecvTimeoutError::Timeout => {}
                RecvTimeoutError::Disconnected => {
                    error!("FATAL: notify_rx hung up in watch_entry");
                    fatal.store(true, Ordering::Relaxed);
                    return bad!("client:watch_entry>> notify_rx disconnected");
                }
            },
        }
    }
}

fn zenoh_entry(
    inode_map: Arc<RwLock<InodeMap>>,
    event_rx: mpsc::Receiver<PathBuf>,
    fatal: Arc<AtomicBool>,
    params: Arc<ClientParameters>,
    watchers: Arc<Mutex<Vec<RecommendedWatcher>>>,
    notify_tx: mpsc::Sender<Event>,
    client_sync: Arc<Mutex<ClientSyncState>>,
    local_dirty: Arc<Mutex<HashSet<PathBuf>>>,
) -> Outcome<()> {
    let (zenoh_client, zenoh_rx, terminal_topic): (ipc::ZenohClient, ipc::Rx, String) =
        match ipc::connect_with_terminate_topic(&[ipc::TOPIC_SERVER], ipc::TOPIC_CLIENTS) {
            Ok(conn) => conn,
            Err(e) => {
                fatal.store(true, Ordering::Relaxed);
                return bad!("Unable to create Zenoh client, {}", e);
            }
        };

    // The server will send status updates to it's clients every 5 seconds
    loop {
        if fatal.load(Ordering::Relaxed) {
            zenoh_client.disconnect();
            info!("client:zenoh_entry>> aborting");
            return Ok(());
        }

        match zenoh_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(message) => {
                if let Err(e) = handle_incoming_transport_message(
                    message,
                    terminal_topic.as_str(),
                    &fatal,
                    &event_rx,
                    &zenoh_client,
                    &inode_map,
                    params.as_ref(),
                    &watchers,
                    &notify_tx,
                    &client_sync,
                    local_dirty.as_ref(),
                ) {
                    error!("client:zenoh_entry>> process: {e}");
                }
            }
            Err(e) => match e {
                RecvTimeoutError::Disconnected => {
                    fatal.store(true, Ordering::Relaxed);
                    return bad!("client:zenoh_entry>> zenoh_rx hung up?");
                }
                RecvTimeoutError::Timeout => {
                    debug!("client:zenoh_entry>> waiting on message...");
                }
            },
        }
    }
}

fn handle_incoming_transport_message(
    message: Option<ipc::ZenohMessage>,
    terminal_topic: &str,
    fatal: &Arc<AtomicBool>,
    event_rx: &mpsc::Receiver<PathBuf>,
    zenoh_client: &ipc::ZenohClient,
    inode_map: &Arc<RwLock<InodeMap>>,
    params: &ClientParameters,
    watchers: &Arc<Mutex<Vec<RecommendedWatcher>>>,
    notify_tx: &mpsc::Sender<Event>,
    client_sync: &Arc<Mutex<ClientSyncState>>,
    local_dirty: &Mutex<HashSet<PathBuf>>,
) -> Outcome<()> {
    let Some(msg) = message else {
        return bad!("client:zenoh_entry>> empty message?");
    };

    if msg.topic == terminal_topic {
        debug!("client:zenoh_entry>> received terminal_topic");
        fatal.store(true, Ordering::Relaxed);
        return Ok(());
    }

    if msg.topic == ipc::TOPIC_CONTROL_RELOAD {
        return apply_client_config_reload(params, inode_map, watchers, notify_tx);
    }

    // process Zenoh traffic from server
    debug!("client>> 👍 recv: {}", msg.payload);
    process(
        event_rx,
        zenoh_client,
        inode_map,
        client_sync,
        local_dirty,
        params,
        &msg.payload,
    )
}

fn process(
    event_rx: &mpsc::Receiver<PathBuf>,
    zenoh_client: &ipc::ZenohClient,
    inode_map: &Arc<RwLock<InodeMap>>,
    client_sync: &Arc<Mutex<ClientSyncState>>,
    local_dirty: &Mutex<HashSet<PathBuf>>,
    params: &ClientParameters,
    server_msg: &ipc::Payload,
) -> Outcome<()> {
    maybe_record_writer_ack(client_sync.as_ref(), server_msg, local_dirty)?;

    match server_msg.status {
        ipc::Status::NotReady(reason) => match reason {
            ipc::Reason::Busy => {
                info!("client:process>> server busy... wait 5 secs");
                std::thread::sleep(Duration::from_secs(5)); // should be config driven
                Ok(())
            }
            ipc::Reason::Behind => {
                debug!("client:process>> Behind synch up");
                if config::is_status_dest(&server_msg.dest_path) {
                    return bad!(
                        "server Behind response missing sync root (dest_path={})",
                        server_msg.dest_path.display()
                    );
                }
                let head = server_msg.head_generation;
                let server_addr = config::server_addr(params)?;
                let sync_root = server_msg.dest_path.clone();
                if let Ok(map_read) = inode_map.read() {
                    let anchor_paths: Vec<PathBuf> = map_read.keys().cloned().collect();
                    let has_dirty = local_dirty.lock().is_ok_and(|d| !d.is_empty());
                    let state_dir = client_state_dir(params);
                    let backup_run = if has_dirty {
                        let dir = conflict::next_behind_backup_dir(&state_dir)?;
                        warn!(
                            "client: behind pull with local edits pending; rsync backups will use {}",
                            dir.display()
                        );
                        Some(dir)
                    } else {
                        None
                    };
                    pull_behind(
                        &server_addr,
                        &sync_root,
                        &anchor_paths,
                        &map_read,
                        backup_run.as_deref(),
                    )?;
                    if let Some(ref dir) = backup_run {
                        info!(
                            "client: behind pull finished; pre-replace copies (if any) are under {} (head_generation={})",
                            dir.display(),
                            head
                        );
                    }
                    let mut payload = ipc::Payload::new()?
                        .status(ipc::Status::NotReady(ipc::Reason::Behind))
                        .src_paths(anchor_paths);
                    attach_client_outbound_basis(&mut payload, client_sync.as_ref())?;
                    zenoh_client.publish(&mut payload)?;
                    record_pull_acked(client_sync.as_ref(), head)?;
                    if let Ok(mut dirty) = local_dirty.lock() {
                        dirty.clear();
                    }
                    Ok(())
                } else {
                    bad!("unable to acquire inode_map read lock")
                }
            }

            ipc::Reason::Other => {
                warn!("client:process>> unhandled NotReady(Other); no action");
                Ok(())
            }
        },
        ipc::Status::Ready => {
            debug!("client:process>> ipc::Status::Ready");
            match filter_file_events(event_rx) {
                Ok(filtered_paths) => {
                    if filtered_paths.is_empty() {
                        debug!("client:process>> nothing to send");
                    } else {
                        let grouped_paths = if let Ok(map_read) = inode_map.read() {
                            let mut grouped: HashMap<String, (RsyncOptions, Vec<PathBuf>)> =
                                HashMap::new();
                            for path in filtered_paths {
                                let rsync_cfg = map_read
                                    .get(&path)
                                    .map_or_else(RsyncOptions::default, |inode| {
                                        inode.rsync_options.clone()
                                    });
                                let key = toml::to_string(&rsync_cfg)
                                    .unwrap_or_else(|_| String::new());
                                grouped
                                    .entry(key)
                                    .or_insert((rsync_cfg, Vec::new()))
                                    .1
                                    .push(path);
                            }
                            grouped.into_values().collect::<Vec<_>>()
                        } else {
                            return bad!("unable to acquire inode_map read lock");
                        };

                        for (rsync_cfg, paths) in grouped_paths {
                            let mut payload =
                                ipc::Payload::new()?.src_paths(paths).rsync(rsync_cfg);
                            attach_client_outbound_basis(&mut payload, client_sync.as_ref())?;
                            if let Err(e) = zenoh_client.publish(&mut payload) {
                                error!("unable to publish {e}");
                            } else {
                                info!("published payload: {payload}");
                            }
                        }
                    }
                    Ok(())
                }
                Err(e) => {
                    bad!("unable to filter_paths: {}", e)
                }
            }
        }
    }
}

fn apply_client_config_reload(
    params: &ClientParameters,
    inode_map: &Arc<RwLock<InodeMap>>,
    watchers: &Arc<Mutex<Vec<RecommendedWatcher>>>,
    notify_tx: &mpsc::Sender<Event>,
) -> Outcome<()> {
    let new_map = config::load_inode_map(params)?;
    let new_watchers = setup_watchers(&new_map, notify_tx.clone())?;
    {
        let mut im = inode_map
            .write()
            .map_err(|e| format!("inode_map write lock poisoned: {e}"))?;
        *im = new_map;
    }
    {
        let mut w = watchers
            .lock()
            .map_err(|e| format!("watchers lock poisoned: {e}"))?;
        *w = new_watchers;
    }
    info!("client: configuration reloaded from disk");
    Ok(())
}

fn setup_watchers(
    inode_map: &InodeMap,
    tx: mpsc::Sender<Event>,
) -> Outcome<Vec<RecommendedWatcher>> {
    let mut watchers: Vec<RecommendedWatcher> = Vec::new();

    for pathbuf in inode_map.keys() {
        // Clone tx for use in this iteration
        let tx_clone = tx.clone();

        // Create a watcher with initial configuration
        let notify_cfg = notify_config_for_platform();
        let mut watcher = RecommendedWatcher::new(
            move |res| match res {
                Ok(event) => {
                    if tx_clone.send(event).is_err() {
                        error!("failed to send notify event");
                    }
                }
                Err(err) => error!("watch error: {err:?}"),
            },
            notify_cfg,
        )
        .map_err(|e| format!("couldn't create watcher: {e}"))?;

        if watcher.watch(pathbuf, RecursiveMode::Recursive).is_err() {
            warn!("unable to set watcher for: '{}'", pathbuf.display());
        } else {
            info!("set watcher for: '{}'", pathbuf.display());
            watchers.push(watcher);
        }
    }

    if watchers.is_empty() {
        bad!("nothing to watch! aborting")
    } else {
        Ok(watchers)
    }
}

// Will loop on file events until queue (channel) is empty
// Using a HashSet to filter out redundancies will return
// sanitized list of paths ready to send to sinkd server
// TODO: need to account for serveral users
fn filter_file_events(event_rx: &mpsc::Receiver<PathBuf>) -> Outcome<Vec<PathBuf>> {
    let mut path_set = HashSet::<PathBuf>::new();
    loop {
        match event_rx.try_recv() {
            // only top level paths are passed
            Ok(path) => {
                debug!("Adding file to path_set");
                path_set.insert(path);
            }
            Err(err) => match err {
                mpsc::TryRecvError::Disconnected => return bad!("event_rx disconnected"),
                mpsc::TryRecvError::Empty => break, // Ready to send!
            },
        }
    }

    Ok(path_set.into_iter().collect())
}

/// Server-side path under the sync root that corresponds to a client anchor (`rsync -aR` layout).
#[must_use]
pub fn server_source_path(anchor: &Path, sync_root: &Path) -> PathBuf {
    let tail = anchor.strip_prefix("/").unwrap_or(anchor);
    sync_root.join(tail)
}

/// Reconcile each watched anchor from the server's sync root (via Zenoh `dest_path`).
fn pull_behind(
    server_addr: &str,
    sync_root: &Path,
    anchor_paths: &[PathBuf],
    inode_map: &InodeMap,
    backup_dir: Option<&Path>,
) -> Outcome<()> {
    for anchor in anchor_paths {
        let source = server_source_path(anchor, sync_root);
        let rsync_cfg = inode_map
            .get(anchor)
            .map(|inode| inode.rsync_options.clone())
            .unwrap_or_default();

        // Late joiners often have no server mirror yet (global generation already
        // advanced by another client). Local and remote must no-op the same way —
        // remote rsync against a missing path exits 23 and would otherwise loop forever.
        let status = check_behind_source(server_addr, &source);
        if skip_missing_behind_source(status) {
            info!(
                "client:pull_behind>> server source {} does not exist yet; nothing to reconcile",
                source.display()
            );
            continue;
        }

        let user = sinkd_core::rsync::rsync_ssh_user();
        let is_local = server_addr.starts_with('/');
        let src = if is_local {
            format!("{}/", source.display())
        } else {
            format!("{user}@{server_addr}:{}/", source.display())
        };
        let dest = format!("{}/", anchor.display());
        debug!("client:pull_behind>> {src} -> {dest} backup:{backup_dir:?}");
        match rsync_behind_mirror_sync(&src, &dest, &rsync_cfg, backup_dir) {
            Ok(()) => {}
            Err(e) => {
                // Race: mirror vanished between probe and rsync.
                if skip_missing_behind_source(check_behind_source(server_addr, &source)) {
                    info!(
                        "client:pull_behind>> server source {} missing after rsync error; treating as empty reconcile ({e})",
                        source.display()
                    );
                } else {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::mpsc};

    use super::filter_file_events;

    #[test]
    fn server_source_path_strips_leading_slash_from_anchor() {
        let anchor = PathBuf::from("/watch/alice");
        let root = PathBuf::from("/srv/sinkd");
        assert_eq!(
            super::server_source_path(&anchor, &root),
            PathBuf::from("/srv/sinkd/watch/alice")
        );
    }

    #[test]
    fn filter_file_events_deduplicates_paths() {
        let (tx, rx) = mpsc::channel();
        let path = PathBuf::from("/tmp/a");
        tx.send(path.clone()).expect("send should succeed");
        tx.send(path.clone()).expect("send should succeed");
        tx.send(PathBuf::from("/tmp/b"))
            .expect("send should succeed");

        let mut filtered = filter_file_events(&rx).expect("filter should succeed");
        filtered.sort();

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0], PathBuf::from("/tmp/a"));
        assert_eq!(filtered[1], PathBuf::from("/tmp/b"));
    }

    #[test]
    fn filter_file_events_returns_error_when_channel_disconnected() {
        let (tx, rx) = mpsc::channel::<PathBuf>();
        drop(tx);

        let err = filter_file_events(&rx).expect_err("disconnect should return an error");
        assert_eq!(err.to_string(), "event_rx disconnected");
    }
}

#[cfg(test)]
mod conflict_resolution_tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use sinkd_core::ipc;

    use crate::sync_state::{mark_local_dirty, maybe_record_writer_ack, ClientSyncState};

    fn sample_payload(last_writer: &str, head_generation: u64) -> ipc::Payload {
        ipc::Payload::from(
            String::from("h"),
            String::from("u"),
            vec![],
            PathBuf::from("sinkd_status"),
            String::from("d"),
            String::new(),
            0,
            head_generation,
            last_writer.to_string(),
            ipc::Status::Ready,
            None,
        )
    }

    #[test]
    fn mark_local_dirty_inserts_resolved_path_for_existing_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("edited.txt");
        fs::write(&file, b"x").expect("write");
        let dirty = Mutex::new(HashSet::<PathBuf>::new());
        mark_local_dirty(&dirty, &file);
        let paths = dirty.lock().expect("lock");
        assert_eq!(paths.len(), 1);
        let p = paths.iter().next().expect("one path");
        assert!(p.ends_with("edited.txt"), "got {}", p.display());
    }

    #[test]
    fn writer_ack_clears_local_dirty_when_last_writer_matches_us() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ack_path = tmp.path().join("acked_generation");
        fs::write(&ack_path, "5").expect("seed ack");
        let sync = Mutex::new(ClientSyncState {
            client_id: "our-id".to_string(),
            acked_generation: 5,
            ack_path,
        });
        let dirty = Mutex::new(HashSet::from([PathBuf::from("/nope/unrelated")]));
        let msg = sample_payload("our-id", 7);
        maybe_record_writer_ack(&sync, &msg, &dirty).expect("ack");
        assert!(dirty.lock().expect("lock").is_empty());
        assert_eq!(sync.lock().expect("lock").acked_generation, 7);
    }

    #[test]
    fn writer_ack_leaves_dirty_set_when_last_writer_is_other_client() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ack_path = tmp.path().join("acked_generation");
        fs::write(&ack_path, "5").expect("seed ack");
        let sync = Mutex::new(ClientSyncState {
            client_id: "our-id".to_string(),
            acked_generation: 5,
            ack_path,
        });
        let marker = PathBuf::from("/tmp/marker");
        let dirty = Mutex::new(HashSet::from([marker.clone()]));
        let msg = sample_payload("other-id", 7);
        maybe_record_writer_ack(&sync, &msg, &dirty).expect("ack");
        assert_eq!(sync.lock().expect("lock").acked_generation, 5);
        assert!(dirty.lock().expect("lock").contains(&marker));
    }

    #[test]
    fn writer_ack_does_not_clear_dirty_when_last_writer_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ack_path = tmp.path().join("acked_generation");
        fs::write(&ack_path, "5").expect("seed ack");
        let sync = Mutex::new(ClientSyncState {
            client_id: "our-id".to_string(),
            acked_generation: 5,
            ack_path,
        });
        let marker = PathBuf::from("/tmp/marker2");
        let dirty = Mutex::new(HashSet::from([marker.clone()]));
        let msg = sample_payload("", 7);
        maybe_record_writer_ack(&sync, &msg, &dirty).expect("ack");
        assert_eq!(sync.lock().expect("lock").acked_generation, 5);
        assert!(dirty.lock().expect("lock").contains(&marker));
    }
}
