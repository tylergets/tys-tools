mod adaptive;
mod ambient;
mod apps;
mod backend;
mod cli;
mod color;
mod config;
mod control;
mod daemon;
mod geoclue;
#[cfg(target_os = "linux")]
mod graph;
mod monitors;
mod protocol;
mod schedule;
mod solar;
#[cfg(target_os = "linux")]
mod tray;

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Result};
use clap::Parser;
use ty_common::status::{Report, StatusArgs};

use crate::cli::{Cli, Commands};
use crate::config::AppConfig;
use crate::monitors::{Change, Monitors};
use crate::protocol::{parse_duration, parse_switch, Command, Status};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run(cli.config),
        Commands::Enable => send(Command::Enable),
        Commands::Disable {
            duration,
            until_morning,
        } => send(match (duration, until_morning) {
            (_, true) => Command::DisableUntilMorning,
            (Some(duration), false) => Command::Disable(Some(parse_duration(&duration)?)),
            (None, false) => Command::Disable(None),
        }),
        Commands::Toggle => send(Command::Toggle),
        Commands::Set { kelvin } => send(Command::Set(kelvin)),
        Commands::Auto => send(Command::Auto),
        Commands::Preset { name } => send(Command::Preset(name)),
        Commands::Presets => {
            for (name, preset) in load_config(cli.config)?.config.presets {
                let mut parts = Vec::new();
                if let Some(kelvin) = preset.temperature {
                    parts.push(format!("{kelvin}K"));
                }
                if let Some(dim) = preset.brightness {
                    parts.push(format!("dimming {dim}"));
                }
                if let Some(percent) = preset.monitor_brightness {
                    parts.push(format!("monitor {percent}%"));
                }
                println!("{name:<16} {}", parts.join(", "));
            }
            Ok(())
        }
        Commands::AutoBrightness { state } => {
            send(Command::AutoBrightness(parse_switch(state.as_deref())?))
        }
        Commands::Graph => send(Command::ShowGraph),
        Commands::Brightness { value } => brightness(cli.config, value),
        Commands::Status(args) => status(args),
        Commands::Location => location(cli.config),
        Commands::ValidateConfig => {
            let loaded = load_config(cli.config)?;
            match loaded.path {
                Some(path) => println!("Config is valid: {}", path.display()),
                None => println!("No config file found; defaults are valid"),
            }
            Ok(())
        }
    }
}

fn load_config(path: Option<PathBuf>) -> Result<ty_common::config::Loaded<AppConfig>> {
    let loaded = AppConfig::load(path)?;
    loaded.config.validate()?;
    Ok(loaded)
}

fn run(config_path: Option<PathBuf>) -> Result<()> {
    println!("tylight v{}", env!("CARGO_PKG_VERSION"));
    let loaded = load_config(config_path)?;
    match &loaded.path {
        Some(path) => println!("Loaded config from {}", path.display()),
        None => println!("No config file found; using defaults"),
    }
    daemon::run(loaded.config)
}

fn send(command: Command) -> Result<()> {
    let status = control::request(&command)?;
    println!("{}", status.format_plain());
    Ok(())
}

fn status(args: StatusArgs) -> Result<()> {
    let fetch = || control::request(&Command::Status).unwrap_or_else(|_| Status::offline());
    if !args.follow {
        fetch().print(args.format);
        return Ok(());
    }
    let mut last = None;
    loop {
        let line = fetch().format(args.format);
        if last.as_ref() != Some(&line) {
            println!("{line}");
            last = Some(line);
        }
        thread::sleep(Duration::from_secs(1));
    }
}

/// Through the daemon when it runs (so the tray stays in sync), else directly.
fn brightness(config_path: Option<PathBuf>, value: Option<String>) -> Result<()> {
    let change = value.as_deref().map(Change::parse).transpose()?;
    let daemon = match change {
        Some(change) => control::request(&Command::Brightness(change)),
        None => control::request(&Command::Status),
    };
    let percent = match daemon {
        Ok(status) => status.monitor_brightness,
        Err(err)
            if err.downcast_ref::<std::io::Error>().is_some_and(|err| {
                matches!(
                    err.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                )
            }) =>
        {
            let config = load_config(config_path)?.config;
            let mut monitors = Monitors::detect(&config.monitor_brightness);
            match change {
                Some(change) => Some(monitors.change(change, monitors::MANUAL_FADE, true)?),
                None => monitors.percent(),
            }
        }
        // A timeout or hardware error must not retry alongside the daemon's DDC writes.
        Err(err) => return Err(err),
    };
    match percent {
        Some(percent) => println!("{percent}%"),
        None => bail!("no controllable displays found"),
    }
    Ok(())
}

fn location(config_path: Option<PathBuf>) -> Result<()> {
    let config = load_config(config_path)?.config;
    let (location, source) = match config.location {
        Some(location) => (location, "config"),
        None => match geoclue::Client::start()?.wait(Duration::from_secs(30))? {
            Some(location) => (location, "geoclue"),
            None => bail!("GeoClue gave no location within 30 seconds"),
        },
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64();
    println!(
        "{:.4}, {:.4} ({source}); sun elevation {:.1}°",
        location.latitude,
        location.longitude,
        solar::elevation(now, location)
    );
    Ok(())
}
