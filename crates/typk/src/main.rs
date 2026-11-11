mod agent;
mod appinfo;
mod cli;
mod config;
mod dialog;
mod fprint;
mod helper;
mod polkit;
mod reasons;
mod users;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands};
use crate::config::AppConfig;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => {
            let config = load_config(cli.config)?;
            println!("typk v{}", env!("CARGO_PKG_VERSION"));
            agent::run(config)
        }
        Commands::Verify {
            reason,
            app,
            action,
        } => match polkit::verify(&action, reason.as_deref(), app.as_deref()) {
            Ok(outcome) => {
                println!("{outcome}");
                std::process::exit(if outcome == polkit::Outcome::Authorized {
                    0
                } else {
                    1
                });
            }
            Err(err) => {
                eprintln!("error: {err:#}");
                std::process::exit(2);
            }
        },
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
