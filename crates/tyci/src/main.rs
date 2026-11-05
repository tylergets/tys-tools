mod cli;
mod config;
mod github;
mod snapshot;
#[cfg(target_os = "linux")]
mod tray;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use ty_common::lock::InstanceLock;
use ty_common::status::{Report, StatusArgs};

use crate::cli::{Cli, Commands};
use crate::config::{AppConfig, APP_NAME};
use crate::snapshot::{ago, Snapshot};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run(cli.config),
        Commands::List { json } => list(cli.config, json),
        Commands::Status(args) => status(cli.config, args),
        Commands::ValidateConfig => {
            let loaded = AppConfig::load(cli.config)?;
            match loaded.path {
                Some(path) => println!("Config is valid: {}", path.display()),
                None => println!("No config file; using defaults"),
            }
            Ok(())
        }
    }
}

fn run(config_path: Option<PathBuf>) -> Result<()> {
    println!("tyci v{}", env!("CARGO_PKG_VERSION"));
    let _lock = InstanceLock::acquire(APP_NAME)?;
    let loaded = AppConfig::load(config_path)?;
    if let Some(path) = &loaded.path {
        println!("Loaded config from {}", path.display());
    }
    #[cfg(target_os = "linux")]
    return tray::run(loaded.config);
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("the tray applet needs Linux");
}

fn list(config_path: Option<PathBuf>, json: bool) -> Result<()> {
    let config = AppConfig::load(config_path)?.config;
    let snapshot = Snapshot::fetch(&config);
    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(());
    }
    let now = Utc::now();
    let section = |title: &str, empty: bool| {
        println!("{title}");
        if empty {
            println!("  (none)");
        }
    };
    section("Running", snapshot.running.is_empty());
    for run in &snapshot.running {
        let time = run
            .started_at
            .map_or_else(|| run.status.clone(), |t| ago(t, now));
        println!(
            "  {} {} · {} · {} · {time}  {}",
            run.mark(),
            run.repo,
            run.workflow,
            run.branch,
            run.url
        );
    }
    section("\nFinished", snapshot.finished.is_empty());
    for run in &snapshot.finished {
        println!(
            "  {} {} · {} · {} · {} ago  {}",
            run.mark(),
            run.repo,
            run.workflow,
            run.branch,
            ago(run.updated_at, now),
            run.url
        );
    }
    section("\nPull requests", snapshot.pull_requests.is_empty());
    for pr in &snapshot.pull_requests {
        let draft = if pr.draft { " (draft)" } else { "" };
        println!(
            "  {}#{} {}{draft} · {} · {}  {}",
            pr.repo,
            pr.number,
            pr.title,
            pr.author,
            ago(pr.created_at, now),
            pr.url
        );
    }
    for err in &snapshot.errors {
        eprintln!("error: {err}");
    }
    Ok(())
}

fn status(config_path: Option<PathBuf>, args: StatusArgs) -> Result<()> {
    let config = AppConfig::load(config_path)?.config;
    let mut last = None;
    loop {
        let summary = Snapshot::fetch(&config).summary();
        if last != Some(summary) {
            summary.print(args.format);
            last = Some(summary);
        }
        if !args.follow {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(config.poll_seconds));
    }
}
