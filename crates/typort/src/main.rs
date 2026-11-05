mod actions;
mod cli;
mod config;
mod ports;
mod title;
#[cfg(target_os = "linux")]
mod tray;

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use serde::Serialize;
use ty_common::status::{Report, StatusArgs};

use crate::cli::{Cli, Commands};
use crate::config::AppConfig;
use crate::ports::Port;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => {
            let config = load_config(cli.config)?;
            println!("typort v{}", env!("CARGO_PKG_VERSION"));
            tray::run(config)
        }
        Commands::List { json, all } => list(&load_config(cli.config)?, json, all),
        Commands::Open { port } => {
            let config = load_config(cli.config)?;
            actions::open(&config, &actions::find(port)?)
        }
        Commands::Kill { port, force } => {
            let found = actions::find(port)?;
            actions::kill(&found, force)?;
            println!("Stopped {} on port {port}", found.label());
            Ok(())
        }
        Commands::Url { port } => {
            let config = load_config(cli.config)?;
            println!("{}", config.url(&actions::find(port)?));
            Ok(())
        }
        Commands::Status(args) => status(&load_config(cli.config)?, args),
        Commands::ValidateConfig => {
            let loaded = AppConfig::load(cli.config)?;
            loaded.config.validate()?;
            match loaded.path {
                Some(path) => println!("Config is valid: {}", path.display()),
                None => println!("No config file found; defaults are valid"),
            }
            Ok(())
        }
    }
}

fn load_config(path: Option<PathBuf>) -> Result<AppConfig> {
    let loaded = AppConfig::load(path)?;
    loaded.config.validate()?;
    Ok(loaded.config)
}

fn list(config: &AppConfig, json: bool, all: bool) -> Result<()> {
    let mut ports = actions::list(config, all)?;
    if config.fetch_titles {
        title::Cache::default().fill(&mut ports, &config.https_ports);
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&ports)?);
        return Ok(());
    }
    if ports.is_empty() {
        println!("No listening ports");
        return Ok(());
    }
    let width = ports
        .iter()
        .map(|p| p.label().chars().count())
        .max()
        .unwrap_or(0)
        .max("PROCESS".len());
    println!(
        "{:<6} {:<7} {:<width$} {:<15} {:<24} TITLE",
        "PORT", "PID", "PROCESS", "REACH", "URL"
    );
    for port in &ports {
        let pid = port
            .processes
            .first()
            .map_or("?".into(), |p| p.pid.to_string());
        println!(
            "{:<6} {:<7} {:<width$} {:<15} {:<24} {}",
            port.port,
            pid,
            port.label(),
            port.reach(),
            config.url(port),
            port.title.as_deref().unwrap_or("")
        );
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct Status {
    ports: Vec<Port>,
}

impl Report for Status {
    fn format_plain(&self) -> String {
        if self.ports.is_empty() {
            return "No listening ports".into();
        }
        let list: Vec<String> = self
            .ports
            .iter()
            .map(|p| format!("{} {}", p.port, p.name()))
            .collect();
        list.join(", ")
    }

    fn format_polybar(&self) -> String {
        let list: Vec<String> = self.ports.iter().map(|p| p.port.to_string()).collect();
        list.join(" ")
    }
}

fn status(config: &AppConfig, args: StatusArgs) -> Result<()> {
    let mut scanner = ports::Scanner::default();
    let mut fetch = || -> Result<Status> {
        Ok(Status {
            ports: actions::list_with(&mut scanner, config, false)?,
        })
    };
    if !args.follow {
        fetch()?.print(args.format);
        return Ok(());
    }
    let mut last = None;
    loop {
        let line = fetch()?.format(args.format);
        if last.as_ref() != Some(&line) {
            println!("{line}");
            last = Some(line);
        }
        thread::sleep(Duration::from_secs_f64(config.refresh_seconds));
    }
}
