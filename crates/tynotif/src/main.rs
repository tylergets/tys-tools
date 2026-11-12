mod bus;
mod config;
mod demo;
mod model;
mod ui;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    version,
    about = "Notification daemon, popup history and do-not-disturb controls"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Replace the current notification daemon if it allows D-Bus replacement.
    #[arg(long, global = true)]
    replace: bool,
    /// Take over, stopping the owning Dunst user service if replacement is refused.
    #[arg(long, global = true)]
    force: bool,
    /// Send demo notifications SECONDS apart (default: 3; range: 1–3600).
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "3",
        value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..=3600))]
    test: Option<u64>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Run,
    ValidateConfig,
    Status,
    List,
    History,
    HistoryClear,
    HistoryPop { id: Option<u32> },
    Close { id: Option<u32> },
    CloseAll,
    IsPaused,
    SetPaused { value: Paused },
    Action { id: u32, key: String },
}

#[derive(Clone, ValueEnum)]
enum Paused {
    True,
    False,
    Toggle,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Run);
    match command {
        Command::Run | Command::ValidateConfig => {
            let config = ty_common::config::load::<config::Config>("tynotif", cli.config)?.config;
            config.validate()?;
            if matches!(command, Command::ValidateConfig) {
                println!("Config is valid");
                return Ok(());
            }
            ui::run(
                Arc::new(Mutex::new(model::State::new(config))),
                cli.replace,
                cli.force,
                cli.test.map(std::time::Duration::from_secs),
            )
        }
        Command::Status => bus::command("status", ""),
        Command::List => bus::command("list", ""),
        Command::History => bus::command("history", ""),
        Command::HistoryClear => bus::command("history-clear", ""),
        Command::HistoryPop { id } => bus::command(
            "history-pop",
            &id.map(|i| i.to_string()).unwrap_or_default(),
        ),
        Command::Close { id } => {
            bus::command("close", &id.map(|i| i.to_string()).unwrap_or_default())
        }
        Command::CloseAll => bus::command("close-all", ""),
        Command::IsPaused => bus::command("is-paused", ""),
        Command::SetPaused { value } => bus::command(
            "set-paused",
            match value {
                Paused::True => "true",
                Paused::False => "false",
                Paused::Toggle => "toggle",
            },
        ),
        Command::Action { id, key } => bus::command("action", &format!("{id}:{key}")),
    }
}
