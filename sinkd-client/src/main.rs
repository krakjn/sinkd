use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use sinkd_client::{load_client, pair, reload_running, run, stop_running, ClientOverrides};
use sinkd_core::{expand, runtime_dir, state_dir};
use sinkd_reflect::{sync_ssh, Opts};

#[derive(Parser)]
#[command(
    name = "sinkd",
    version,
    propagate_version = true,
    about = "Watch folders and sync them to a sinkd server"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    runtime_dir: Option<PathBuf>,
    /// Implies `run` when no subcommand is given.
    #[arg(long, global = true)]
    server: Option<String>,
    #[arg(long, global = true)]
    anchor: Vec<PathBuf>,
    #[arg(long, global = true)]
    share: Vec<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Exchange keys with a running server. Type the code printed on the server.
    Pair {
        host: String,
        #[arg(long)]
        code: Option<String>,
        #[arg(long)]
        pair_port: Option<u16>,
    },
    Stop,
    Reload,
    /// One-shot copy over SSH using libreflect. `remote` is user@host:/path/.
    Ssh {
        remote: String,
        #[arg(long)]
        anchor: PathBuf,
        #[arg(long)]
        delete: bool,
    },
}

fn unpaired(err: &str) -> bool {
    err.starts_with("no server set") || err.contains("not paired yet") || err.starts_with("no anchors")
}

// true to try again, false when the operator hits Ctrl-C.
async fn pause_for_pair(err: &str) -> bool {
    log::info!("{err}");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => false,
        () = tokio::time::sleep(Duration::from_secs(30)) => true,
    }
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    let runtime = cli.runtime_dir.clone().unwrap_or_else(runtime_dir);
    let state = cli.state_dir.clone().unwrap_or_else(|| state_dir("sinkd"));
    let result = match cli.cmd {
        Some(Cmd::Pair { host, code, pair_port }) => {
            let port = pair_port.unwrap_or(7449);
            let rt = tokio::runtime::Runtime::new().expect("runtime");
            rt.block_on(pair(&state, &host, port, code))
        }
        Some(Cmd::Stop) => stop_running(&runtime),
        Some(Cmd::Reload) => reload_running(&runtime),
        Some(Cmd::Ssh { remote, anchor, delete }) => {
            let rules = sinkd_core::Rules::load(&anchor, &[]);
            sync_ssh(&anchor, &remote, Opts { delete, update_only: false }, rules).map(|_| ())
        }
        None => {
            let anchors: Vec<_> = cli.anchor.iter().map(|p| expand(&p.to_string_lossy())).collect();
            let shares: Vec<_> = cli.share.iter().map(|p| expand(&p.to_string_lossy())).collect();
            let rt = tokio::runtime::Runtime::new().expect("runtime");
            rt.block_on(async {
                loop {
                    let loaded = load_client(
                        cli.config.as_deref(),
                        ClientOverrides {
                            server: cli.server.clone(),
                            anchors: anchors.clone(),
                            shares: shares.clone(),
                            state_dir: Some(state.clone()),
                            runtime_dir: Some(runtime.clone()),
                            ..ClientOverrides::default()
                        },
                    );
                    match loaded {
                        Err(e) if unpaired(&e) => {
                            if !pause_for_pair(&e).await {
                                return Ok(());
                            }
                        }
                        Err(e) => return Err(e),
                        Ok(settings) => match run(settings).await {
                            Err(e) if unpaired(&e) => {
                                if !pause_for_pair(&e).await {
                                    return Ok(());
                                }
                            }
                            other => return other,
                        },
                    }
                }
            })
        }
    };
    if let Err(e) = result {
        eprintln!("sinkd: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
