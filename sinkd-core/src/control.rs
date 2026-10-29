use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, watch};

pub fn socket_path(runtime: &Path, name: &str) -> PathBuf {
    runtime.join(name)
}

pub async fn serve(
    path: PathBuf,
    stop_tx: watch::Sender<bool>,
    mut stop_rx: watch::Receiver<bool>,
    reload: mpsc::UnboundedSender<()>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let _ = std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    log::info!("control socket {}", path.display());
    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(pair) => pair,
                    Err(e) => {
                        log::warn!("control accept: {e}");
                        continue;
                    }
                };
                let stop_tx = stop_tx.clone();
                let reload = reload.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stream);
                    let mut line = String::new();
                    if lines.read_line(&mut line).await.is_err() {
                        return;
                    }
                    match line.trim() {
                        "stop" => {
                            let _ = stop_tx.send(true);
                        }
                        "reload" => {
                            let _ = reload.send(());
                        }
                        other => {
                            let _ = lines.write_all(format!("unknown {other}\n").as_bytes()).await;
                        }
                    }
                });
            }
        }
    }
    let _ = std::fs::remove_file(&path);
    Ok(())
}

pub fn notify(path: &Path, cmd: &str) -> Result<(), String> {
    use std::io::Write;
    let mut stream = std::os::unix::net::UnixStream::connect(path)
        .map_err(|e| format!("{}: {e} (is it running?)", path.display()))?;
    stream
        .write_all(format!("{cmd}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(())
}
