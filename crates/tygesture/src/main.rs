//! tygesture: touchpad gestures from libinput, mapped to shell commands.

mod config;
mod gesture;
mod gui;
mod report;
mod state;
mod touchpad;
mod tray;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use config::Config;
use gesture::{Gesture, Output, Raw, Recognizer};
use report::DaemonStatus;
use touchpad::Item;
use ty_common::status::StatusArgs;
use ty_common::wake::Waker;

const APP: &str = "tygesture";

#[derive(Parser)]
#[command(
    version,
    about = "Touchpad gestures from libinput, mapped to shell commands"
)]
struct Cli {
    /// Config file (default: ./tygesture.yaml, then ~/.config/tygesture/config.yaml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the gesture daemon (the default).
    Run,
    /// Print gestures as you make them, without running any commands.
    Debug {
        /// Also print every update event with its raw motion.
        #[arg(long)]
        updates: bool,
    },
    /// Validate the config and list its bindings.
    Check,
    /// Open the gesture window of the running daemon.
    Show,
    /// Print the daemon's status (for bar modules).
    Status(StatusArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(Cmd::Status(args)) = &cli.command {
        args.run(Duration::from_secs(1), || {
            ty_common::status::request::<DaemonStatus>(APP).unwrap_or_default()
        });
        return Ok(());
    }
    let loaded = ty_common::config::load::<Config>(APP, cli.config)?;
    let cfg = loaded.config;
    match cli.command.unwrap_or(Cmd::Run) {
        Cmd::Check => {
            match &loaded.path {
                Some(path) => println!("config: {}", path.display()),
                None => println!("config: none found, using defaults"),
            }
            let bindings = cfg.bindings();
            if bindings.is_empty() {
                println!("no gestures bound");
            }
            for (gesture, binding) in bindings {
                let repeat = if binding.repeat { " (repeat)" } else { "" };
                println!("{:<16} {}{repeat}", gesture.to_string(), binding.command);
            }
            Ok(())
        }
        Cmd::Run => run(&cfg),
        Cmd::Show => show(),
        Cmd::Status(_) => unreachable!("handled above"),
        Cmd::Debug { updates } => {
            println!("make some gestures (ctrl+c to quit)");
            listen_debug(&cfg, updates)
        }
    }
}

/// The daemon: run bound commands, with a tray icon unless `tray: false`.
fn run(cfg: &Config) -> Result<()> {
    let lock = ty_common::lock::InstanceLock::acquire(APP)?;
    if cfg.bindings().is_empty() {
        eprintln!("{APP}: no gestures bound; see `{APP} check`");
    }
    let status: state::Shared = Arc::new(Mutex::new(state::Status::new(cfg)));
    let (waker, wakeups) = ty_common::wake::pair();
    let _tray = cfg
        .tray
        .then(|| tray::start(status.clone(), wakeups))
        .flatten();
    serve(lock.listener(), status.clone(), waker.clone())?;

    let mut recognizer = Recognizer::default();
    touchpad::run(|item| match item {
        Item::Touchpad(name, added) => {
            eprintln!(
                "{APP}: touchpad {}: {name}",
                if added { "added" } else { "removed" }
            );
            status.lock().unwrap().touchpad = added.then_some(name);
            waker.wake();
        }
        Item::Gesture(raw) => {
            status.lock().unwrap().track(raw);
            for output in recognizer.feed(cfg, raw) {
                let Output::Fire(gesture) = output else {
                    continue;
                };
                let Some(binding) = cfg.binding(gesture) else {
                    continue;
                };
                let mut status = status.lock().unwrap();
                if status.paused {
                    continue;
                }
                status.fired(gesture.to_string(), binding.command.clone());
                drop(status);
                waker.wake();
                eprintln!("{gesture}: {}", binding.command);
                spawn(&binding.command, gesture);
            }
        }
    })
}

fn listen_debug(cfg: &Config, updates: bool) -> Result<()> {
    let mut recognizer = Recognizer::default();
    touchpad::run(|item| match item {
        Item::Touchpad(name, added) => {
            println!(
                "touchpad {}: {name}",
                if added { "added" } else { "removed" }
            );
        }
        Item::Gesture(raw) => {
            if updates || !matches!(raw, Raw::SwipeUpdate { .. } | Raw::PinchUpdate { .. }) {
                println!("  {raw:?}");
            }
            for output in recognizer.feed(cfg, raw) {
                match output {
                    Output::Fire(gesture) => match cfg.binding(gesture) {
                        Some(binding) => println!("=> {gesture}: {}", binding.command),
                        None => println!("=> {gesture} (unbound)"),
                    },
                    Output::Ended(summary) => println!("{summary}\n"),
                }
            }
        }
    })
}

/// Answer `tygesture show` on the instance socket.
/// Answer `show` and `status` on the instance socket.
fn serve(
    listener: &std::os::unix::net::UnixListener,
    status: state::Shared,
    waker: Waker,
) -> Result<()> {
    ty_common::lock::serve(listener, move |line| match line {
        "show" => {
            status.lock().unwrap().show_window = true;
            waker.wake();
            "ok".into()
        }
        "status" => ty_common::status::to_reply(&DaemonStatus::of(&status.lock().unwrap())),
        _ => "unknown command".into(),
    })
}

fn show() -> Result<()> {
    let reply = ty_common::lock::request(APP, "show")?;
    if reply != "ok" {
        bail!("{APP}: {reply}");
    }
    Ok(())
}

/// Run `command` with `sh -c`, without waiting. `$TYGESTURE` holds the gesture name.
fn spawn(command: &str, gesture: Gesture) {
    let child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .env("TYGESTURE", gesture.to_string())
        .stdin(Stdio::null())
        .spawn();
    match child {
        // Reap it in the background so repeat gestures don't leave zombies.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => eprintln!("{APP}: failed to run {command:?}: {err}"),
    }
}
