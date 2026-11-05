mod cli;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::Parser;
use gtk::glib::prelude::*;
use tyspeak::config::AppConfig;
use tyspeak::core::dbus;
use tyspeak::core::status::Status;
use tyspeak::speech::audio;
use tyspeak::speech::download::{self, Progress};
use tyspeak::speech::engine::Engine;
use tyspeak::speech::models::{self, human_size, MODELS};

use ty_common::status::{Report, StatusArgs};

use crate::cli::{Cli, Commands};

fn main() -> Result<()> {
    let cli = Cli::parse();
    // transcribe.cpp and ggml log every load and decode to stderr.
    if std::env::var_os("TYSPEAK_DEBUG").is_none() {
        transcribe_cpp::disable_logging();
    }

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => run(cli.config),
        Commands::Start => call("Start"),
        Commands::Stop => call("Stop"),
        Commands::Toggle => call("Toggle"),
        Commands::Cancel => call("Cancel"),
        Commands::Status(args) => status(args),
        Commands::Models => list_models(cli.config),
        Commands::Model { id } => {
            if models::find(&id).is_none() {
                bail!("unknown model '{id}' (see `tyspeak models`)");
            }
            dbus::call("SetModel", Some((id,).to_variant()))?;
            Ok(())
        }
        Commands::Retry => call("Retry"),
        Commands::Transcribe { file, model } => transcribe(cli.config, &file, model),
        Commands::ValidateConfig => validate_config(cli.config),
    }
}

fn load_config(path: Option<PathBuf>) -> Result<AppConfig> {
    let loaded = AppConfig::load(path)?;
    loaded.config.validate()?;
    Ok(loaded.config)
}

fn run(config_path_override: Option<PathBuf>) -> Result<()> {
    println!("tyspeak v{}", env!("CARGO_PKG_VERSION"));
    let loaded = AppConfig::load(config_path_override)?;
    loaded.config.validate()?;
    match &loaded.path {
        Some(path) => println!("Loaded config from {}", path.display()),
        None => println!("No config file found; using defaults"),
    }

    #[cfg(target_os = "linux")]
    tyspeak::ui::tray::run(loaded.config)?;
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("tyspeak only supports Linux/X11");

    Ok(())
}

fn call(method: &str) -> Result<()> {
    dbus::call(method, None)?;
    Ok(())
}

fn status(args: StatusArgs) -> Result<()> {
    if !args.follow {
        dbus::status()
            .unwrap_or_else(|_| Status::offline())
            .print(args.format);
        return Ok(());
    }
    let format = args.format;
    dbus::follow(move |status| status.print(format))
}

#[derive(serde::Deserialize)]
struct Listing {
    id: String,
    downloaded: bool,
    selected: bool,
    active: bool,
}

/// The catalog, marked with what the daemon reports (or what is on disk when it isn't
/// running).
fn list_models(config_path: Option<PathBuf>) -> Result<()> {
    let daemon: Option<Vec<Listing>> = dbus::call("ListModels", None)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok());
    let dir = load_config(config_path)?.resolved_model_dir()?;
    for model in MODELS {
        let listed = daemon
            .as_ref()
            .and_then(|list| list.iter().find(|entry| entry.id == model.id));
        let downloaded = listed.map_or_else(|| model.is_downloaded(&dir), |entry| entry.downloaded);
        let mark = match listed {
            Some(entry) if entry.active => "*",
            Some(entry) if entry.selected => ">",
            _ => " ",
        };
        println!(
            "{mark} {:<24} {:>7}  {:<14} {}",
            model.id,
            model.size_label(),
            if downloaded { "downloaded" } else { "-" },
            model.summary
        );
    }
    if daemon.is_some() {
        println!("\n* loaded   > selected (loading or downloading)");
    }
    Ok(())
}

fn transcribe(config_path: Option<PathBuf>, file: &Path, model: Option<String>) -> Result<()> {
    let config = load_config(config_path)?;
    let id = model.unwrap_or_else(|| config.model.clone());
    let model = models::find(&id).with_context(|| format!("unknown model '{id}'"))?;
    let dir = config.resolved_model_dir()?;

    if !model.is_downloaded(&dir) {
        eprintln!("downloading {} ({})…", model.name, model.size_label());
        let progress = Arc::new(Progress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = {
            let (progress, cancel, dir) = (progress.clone(), cancel.clone(), dir.clone());
            thread::spawn(move || download::fetch(model, &dir, &progress, &cancel))
        };
        while !worker.is_finished() {
            eprint!(
                "\r  {:.0}% ({} / {})   ",
                progress.fraction() * 100.0,
                human_size(progress.done()),
                human_size(progress.total())
            );
            thread::sleep(Duration::from_millis(500));
        }
        eprintln!();
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("download thread panicked"))??;
    }

    let mut reader = hound::WavReader::open(file)
        .with_context(|| format!("failed to open {}", file.display()))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };
    let pcm = audio::to_model_input(&samples, usize::from(spec.channels), spec.sample_rate);

    let engine = Engine::default();
    let started = Instant::now();
    engine.load(
        engine.next_generation(),
        model.id,
        &model.path_in(&dir),
        config.threads,
    )?;
    eprintln!("loaded in {:.1}s", started.elapsed().as_secs_f32());
    let started = Instant::now();
    let text = engine.transcribe(&pcm, config.language.as_deref())?;
    let seconds = pcm.len() as f32 / audio::SAMPLE_RATE as f32;
    let took = started.elapsed().as_secs_f32();
    eprintln!(
        "transcribed {seconds:.1}s of audio in {took:.2}s ({:.0}x realtime)",
        seconds / took.max(0.001)
    );
    println!("{text}");
    Ok(())
}

fn validate_config(config_path_override: Option<PathBuf>) -> Result<()> {
    let loaded = AppConfig::load(config_path_override)?;
    loaded.config.validate()?;
    match &loaded.path {
        Some(path) => println!("Config is valid: {}", path.display()),
        None => println!("No config file found; defaults are valid"),
    }
    Ok(())
}
