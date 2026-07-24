mod compose;
mod manifest;
mod output;
mod runner;
mod spec;

use clap::{Parser, Subcommand};
use compose::cli::{ComposeCommand, ComposeRunArgs};
use log::{error, info};
use runner::ScenarioRunner;
use spec::parse_spec;
use std::{fs, path::PathBuf, process::exit};

const SPECS_DIR: &str = "scenario/specs"; // relative to repo root; resolved in discover_specs()

#[derive(Parser, Debug)]
#[command(name = "scenario")]
#[command(about = "Scenario test harness: TOML specs and compose helpers")]
struct Cli {
    #[command(subcommand)]
    command: Option<TopCommand>,
    #[command(flatten)]
    run: RunArgs,
}

#[derive(Subcommand, Debug)]
enum TopCommand {
    /// Compose scenario helpers (manifests, generation checks, event definitions).
    Compose {
        #[command(subcommand)]
        command: ComposeCommand,
    },
}

#[derive(Parser, Debug)]
struct RunArgs {
    /// Path to a single scenario spec TOML file (default: run all specs in scenario/specs/)
    #[arg(short, long)]
    spec: Option<PathBuf>,
    /// List available scenario specs and exit
    #[arg(long)]
    list: bool,
    /// Root directory for file-based scenario steps (default: scenario/output/<spec>/workspace)
    #[arg(short, long)]
    root: Option<PathBuf>,
}

fn discover_specs() -> Result<Vec<PathBuf>, String> {
    let repo = output::find_repo_root().ok_or("cannot locate repo root (expected pkg/debian/create.sh)")?;
    let dir = repo.join(SPECS_DIR);
    if !dir.is_dir() {
        return Err(format!("specs directory not found: {}", dir.display()));
    }
    let mut specs: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| format!("failed to read '{}': {e}", dir.display()))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            path.extension().is_some_and(|ext| ext == "toml").then_some(path)
        })
        .collect();
    specs.sort();
    if specs.is_empty() {
        return Err(format!("no .toml specs found in {}", dir.display()));
    }
    Ok(specs)
}

fn run_spec(spec_path: &PathBuf, root_override: Option<&PathBuf>) -> Result<(), String> {
    let contents = fs::read_to_string(spec_path)
        .map_err(|e| format!("unable to read spec '{}': {e}", spec_path.display()))?;
    let spec = parse_spec(&contents)?;
    let root = match root_override {
        Some(r) => r.clone(),
        None => output::spec_workspace_dir(&spec.name)?,
    };
    info!(
        "running scenario '{}' from '{}' at root '{}'",
        spec.name,
        spec_path.display(),
        root.display()
    );
    let runner = ScenarioRunner::new(root);
    runner.run(&spec)
}

fn run_specs(args: RunArgs) -> ! {
    if args.list {
        match discover_specs() {
            Ok(specs) => {
                for spec in specs {
                    println!("{}", spec.display());
                }
            }
            Err(e) => {
                error!("{e}");
                exit(1);
            }
        }
        exit(0);
    }

    let specs = match args.spec {
        Some(path) => vec![path],
        None => match discover_specs() {
            Ok(specs) => specs,
            Err(e) => {
                error!("{e}");
                exit(1);
            }
        },
    };

    let mut failed = false;
    for spec_path in &specs {
        if let Err(e) = run_spec(spec_path, args.root.as_ref()) {
            error!("scenario '{}' failed: {e}", spec_path.display());
            failed = true;
        }
    }

    if failed {
        exit(1);
    }
    exit(0);
}

fn main() {
    env_logger::init();
    let cli = Cli::parse();

    match cli.command {
        Some(TopCommand::Compose { command }) => {
            if let Err(e) = compose::cli::run(ComposeRunArgs { command }) {
                error!("{e}");
                exit(1);
            }
            exit(0);
        }
        None => run_specs(cli.run),
    }
}
