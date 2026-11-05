mod cli;
mod config;
mod core;
mod io;
mod platform;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use ty_common::lock::InstanceLock;

use crate::cli::{Cli, Commands};
use crate::config::{AppConfig, APP_NAME};
use crate::core::engine::Engine;
#[cfg(target_os = "linux")]
use crate::platform::app_indicator;
use crate::platform::keyboard::KeyboardBackend;
use crate::platform::notify;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run(cli.config, cli.debug),
        Commands::ValidateConfig => validate_config(cli.config),
    }
}

fn run(config_path_override: Option<std::path::PathBuf>, debug: bool) -> Result<()> {
    println!("tykey v{}", env!("CARGO_PKG_VERSION"));
    let _instance_lock = InstanceLock::acquire(APP_NAME)?;

    let loaded = AppConfig::load(config_path_override)?;
    let config_path = loaded.path.clone();
    let watch = loaded.config.watch;
    let config = loaded.config;
    config.validate()?;

    println!("Loaded config from {}", config_path.display());
    let session = ty_common::session::require("tykey")?;
    println!("Listening for keys ({session:?})...");

    #[cfg(target_os = "linux")]
    let _app_indicator = app_indicator::start(
        config.snippets.clone(),
        config.globals.clone(),
        config.notifications.clone(),
    );

    let backend = Arc::new(KeyboardBackend::new()?);
    let mut engine = Engine::new(config);
    engine.set_debug(debug);
    engine.set_output(backend.clone());
    let engine = Arc::new(Mutex::new(engine));

    if watch {
        println!("Watching config for changes: {}", config_path.display());
        start_config_watcher(config_path, Arc::clone(&engine));
    }

    backend.listen(move |event| {
        let mut guard = engine.lock().expect("engine mutex poisoned");
        if let Err(err) = guard.handle_event(event) {
            eprintln!("event handling error: {err}");
            notify("Expansion Error", &err.to_string());
        }
    })?;

    Ok(())
}

fn validate_config(config_path_override: Option<std::path::PathBuf>) -> Result<()> {
    let loaded = AppConfig::load(config_path_override)?;
    loaded.config.validate()?;
    println!("Config is valid: {}", loaded.path.display());
    Ok(())
}

fn start_config_watcher(config_path: PathBuf, engine: Arc<Mutex<Engine>>) {
    std::thread::spawn(move || {
        let mut last_seen_contents = std::fs::read_to_string(&config_path).unwrap_or_default();

        loop {
            std::thread::sleep(Duration::from_secs(1));

            let current_contents = match std::fs::read_to_string(&config_path) {
                Ok(contents) => contents,
                Err(err) => {
                    eprintln!("failed to read config while watching: {err}");
                    continue;
                }
            };

            if current_contents == last_seen_contents {
                continue;
            }

            match AppConfig::load(Some(config_path.clone())) {
                Ok(loaded) => {
                    if let Err(err) = loaded.config.validate() {
                        eprintln!("config changed but validation failed: {err}");
                        last_seen_contents = current_contents;
                        continue;
                    }

                    let mut guard = engine.lock().expect("engine mutex poisoned");
                    guard.reload_config(loaded.config);
                    println!("Reloaded config from {}", config_path.display());
                }
                Err(err) => {
                    eprintln!("config changed but reload failed: {err}");
                }
            }

            last_seen_contents = current_contents;
        }
    });
}
