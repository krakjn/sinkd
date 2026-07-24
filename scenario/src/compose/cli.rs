use std::{fs, io, path::PathBuf};

use clap::{Args, Subcommand};
use log::info;

use crate::{compose, manifest};

#[derive(Subcommand, Debug)]
pub enum ComposeCommand {
    /// Emit a deterministic sha256 manifest for a local directory tree.
    SnapshotDir {
        /// Directory to snapshot (host path).
        dir: PathBuf,
    },
    /// Write a manifest for a local directory to a file.
    WriteManifest {
        dir: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Compare two manifest files; exit 1 on drift.
    CompareManifests {
        #[arg(long)]
        client: PathBuf,
        #[arg(long)]
        server: PathBuf,
        #[arg(long)]
        user: String,
        #[arg(long)]
        label: String,
    },
    /// Print `current_generation` from a `generation_state.toml` file or stdin.
    GetGeneration {
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Exit 0 when `current_generation` >= minimum.
    CheckGeneration {
        #[arg(long)]
        minimum: u64,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Print phase-B operations as tab-separated lines for compose.sh.
    PhaseBOps,
}

#[derive(Args, Debug)]
pub struct ComposeRunArgs {
    #[command(subcommand)]
    pub command: ComposeCommand,
}

pub fn run(args: ComposeRunArgs) -> Result<(), String> {
    match args.command {
        ComposeCommand::SnapshotDir { dir } => {
            manifest::generate_to_stdout(&dir).map_err(|e| e.to_string())?;
        }
        ComposeCommand::WriteManifest { dir, output } => {
            manifest::write_manifest(&dir, &output)?;
        }
        ComposeCommand::CompareManifests {
            client,
            server,
            user,
            label,
        } => {
            manifest::compare_files(&client, &server).map_err(|e| {
                format!(
                    "{label} drift for {user}\n  client: {}\n  server: {}\n{e}",
                    client.display(),
                    server.display()
                )
            })?;
            info!("{label} {user}: tree equivalent");
        }
        ComposeCommand::GetGeneration { file } => {
            let text = read_input(file.as_deref())?;
            match compose::generation::parse_current_generation(&text) {
                Some(n) => println!("{n}"),
                None => return Err("current_generation not found".into()),
            }
        }
        ComposeCommand::CheckGeneration { minimum, file } => {
            let text = read_input(file.as_deref())?;
            let cur = compose::generation::check_minimum(&text, minimum)?;
            info!("current_generation={cur} (>= {minimum})");
        }
        ComposeCommand::PhaseBOps => {
            for line in compose::events::phase_b_script_lines() {
                println!("{line}");
            }
        }
    }
    Ok(())
}

fn read_input(file: Option<&std::path::Path>) -> Result<String, String> {
    match file {
        Some(path) => fs::read_to_string(path)
            .map_err(|e| format!("read '{}': {e}", path.display())),
        None => io::read_to_string(io::stdin()).map_err(|e| format!("read stdin: {e}")),
    }
}
