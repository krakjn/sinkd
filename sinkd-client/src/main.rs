use std::path::PathBuf;
use std::process::ExitCode;

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
            sync_ssh(&anchor, &remote, Opts { delete, update_only: false }).map(|_| ())
        }
        None => {
            let settings = load_client(
                cli.config.as_deref(),
                ClientOverrides {
                    server: cli.server,
                    anchors: cli.anchor.into_iter().map(|p| expand(&p.to_string_lossy())).collect(),
                    shares: cli.share.into_iter().map(|p| expand(&p.to_string_lossy())).collect(),
                    state_dir: Some(state),
                    runtime_dir: Some(runtime),
                    ..ClientOverrides::default()
                },
            );
            match settings {
                Ok(settings) => {
                    let rt = tokio::runtime::Runtime::new().expect("runtime");
                    rt.block_on(run(settings))
                }
                Err(e) => Err(e),
            }
        }
    };
    if let Err(e) = result {
        eprintln!("sinkd: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
