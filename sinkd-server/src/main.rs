use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use sinkd_core::{expand, load_server, notify, runtime_dir, socket_path, ServerOverrides};

#[derive(Parser)]
#[command(name = "sinkd-srv", about = "Sinkd sync server. Stays in the foreground.")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    runtime_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    sync_root: Option<PathBuf>,
    #[arg(long, global = true)]
    bind: Option<String>,
    #[arg(long, global = true)]
    zenoh_port: Option<u16>,
    #[arg(long, global = true)]
    data_port: Option<u16>,
    #[arg(long, global = true)]
    pair_port: Option<u16>,
    /// Fixed pair code, for scripts. Leave unset so the server prints a random code.
    #[arg(long, global = true)]
    pair_code: Option<String>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    Stop,
    Reload,
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    let runtime = cli.runtime_dir.clone().unwrap_or_else(runtime_dir);
    let result = match cli.cmd {
        Some(Cmd::Stop) => notify(&socket_path(&runtime, "sinkd-srv.sock"), "stop"),
        Some(Cmd::Reload) => notify(&socket_path(&runtime, "sinkd-srv.sock"), "reload"),
        None => {
            let settings = load_server(
                cli.config.as_deref(),
                ServerOverrides {
                    sync_root: cli.sync_root.map(|p| expand(&p.to_string_lossy())),
                    bind: cli.bind,
                    zenoh_port: cli.zenoh_port,
                    data_port: cli.data_port,
                    pair_port: cli.pair_port,
                    state_dir: cli.state_dir,
                    runtime_dir: Some(runtime),
                    fixed_pair_code: cli.pair_code,
                },
            );
            match settings {
                Ok(settings) => {
                    let rt = tokio::runtime::Runtime::new().expect("runtime");
                    rt.block_on(sinkd_server::run(settings))
                }
                Err(e) => Err(e),
            }
        }
    };
    if let Err(e) = result {
        eprintln!("sinkd-srv: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
