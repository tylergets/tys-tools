mod cli;

use std::thread;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use ty_common::lock::InstanceLock;
use ty_common::status::{Report, StatusArgs};
use tycapture::config::AppConfig;
use tycapture::core::control::{self, Action, Request};
use tycapture::core::status::Status;

use crate::cli::{Cli, Commands, StartArgs};

/// How long `status --follow` waits before reconnecting to a stopped applet.
const FOLLOW_RETRY_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run(cli.config),
        Commands::Start(StartArgs { target, now }) => send(Action::Start {
            target,
            skip_dialog: now,
        }),
        Commands::Stop => send(Action::Stop),
        Commands::Toggle(StartArgs { target, now }) => send(Action::Toggle {
            target,
            skip_dialog: now,
        }),
        Commands::Status(args) => status(args),
        Commands::ValidateConfig => validate_config(cli.config),
    }
}

fn run(config_path_override: Option<std::path::PathBuf>) -> Result<()> {
    println!("tycapture v{}", env!("CARGO_PKG_VERSION"));
    let instance_lock = InstanceLock::acquire("tycapture")?;

    let loaded = AppConfig::load(config_path_override)?;
    loaded.config.validate()?;
    match &loaded.path {
        Some(path) => println!("Loaded config from {}", path.display()),
        None => println!("No config file found; using defaults"),
    }

    #[cfg(target_os = "linux")]
    tycapture::ui::tray::run(loaded.config, instance_lock.listener().try_clone()?)?;
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("tycapture only supports Linux/X11");

    drop(instance_lock);
    Ok(())
}

fn send(action: Action) -> Result<()> {
    control::request(&Request::Action(action))?;
    Ok(())
}

fn status(args: StatusArgs) -> Result<()> {
    if !args.follow {
        control::request(&Request::Status)
            .ok()
            .and_then(|response| response.status)
            .unwrap_or_else(Status::offline)
            .print(args.format);
        return Ok(());
    }

    loop {
        // Errors just mean the applet isn't running (yet); report offline and retry.
        let _ = control::subscribe(|status| status.print(args.format));
        Status::offline().print(args.format);
        thread::sleep(FOLLOW_RETRY_INTERVAL);
    }
}

fn validate_config(config_path_override: Option<std::path::PathBuf>) -> Result<()> {
    let loaded = AppConfig::load(config_path_override)?;
    loaded.config.validate()?;
    match &loaded.path {
        Some(path) => println!("Config is valid: {}", path.display()),
        None => println!("No config file found; defaults are valid"),
    }
    Ok(())
}
