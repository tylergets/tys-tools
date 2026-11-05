//! Tray applet: one submenu per port, to open, copy or stop it. Ports are scanned on a
//! background thread so a slow `/proc` walk never stalls the menu.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use gtk::glib::{self, ControlFlow};
use ty_common::icons::Tool;
use ty_common::lock::InstanceLock;
use ty_common::notify;
use ty_common::tray::{self, Applet, IconState, MenuEntry, TraySpec};

use crate::actions;
use crate::config::{AppConfig, APP_NAME};
use crate::ports::{Port, Scanner};
use crate::title;

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(3);
/// Longest command line shown in a menu.
const MAX_COMMAND_CHARS: usize = 70;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    Open(u16),
    CopyUrl(u16),
    Kill(u16),
    ForceKill(u16),
    KillAll,
}

pub fn run(config: AppConfig) -> Result<()> {
    let _lock = InstanceLock::acquire(APP_NAME)?;
    gtk::init().context("failed to initialize GTK")?;

    let (ports_tx, ports_rx) = tray::channel();
    let (rescan_tx, rescan_rx) = mpsc::channel();
    spawn_scanner(config.clone(), ports_tx, rescan_rx);

    let (actions_tx, actions_rx) = tray::channel();
    let mut applet = Applet::new(
        APP_NAME,
        &Tool::Port.tray_icons(),
        spec(&config, &[]),
        actions_tx,
    )?;

    // The ports in the menu, for the action handler.
    let shown: Rc<RefCell<Vec<Port>>> = Rc::default();
    let refresh_config = config.clone();
    let shown_by_scan = shown.clone();
    tray::drain_actions(ports_rx, move |scan: Vec<Port>| {
        applet.update(spec(&refresh_config, &scan));
        shown_by_scan.replace(scan);
        ControlFlow::Continue
    });

    tray::drain_actions(actions_rx, move |action| {
        handle(&config, &shown.borrow(), action);
        // Rescan now, and again once the process had time to exit.
        let _ = rescan_tx.send(());
        let rescan = rescan_tx.clone();
        glib::timeout_add_local_once(Duration::from_millis(600), move || {
            let _ = rescan.send(());
        });
        ControlFlow::Continue
    });

    gtk::main();
    Ok(())
}

/// Scan every `refresh_seconds` (or when asked) and send the ports when they change.
fn spawn_scanner(config: AppConfig, ports: tray::Sender<Vec<Port>>, rescan: Receiver<()>) {
    let interval = Duration::from_secs_f64(config.refresh_seconds);
    thread::spawn(move || {
        let mut last = None;
        let mut titles = title::Cache::default();
        let mut scanner = Scanner::default();
        loop {
            let scan = actions::list_with(&mut scanner, &config, false).map(|mut ports| {
                titles.fill(&mut ports, &config.https_ports);
                ports
            });
            match scan {
                Ok(scan) if last.as_ref() != Some(&scan) => {
                    if ports.send_blocking(scan.clone()).is_err() {
                        return;
                    }
                    last = Some(scan);
                }
                Ok(_) => {}
                Err(err) => eprintln!("scan failed: {err:#}"),
            }
            match rescan.recv_timeout(interval) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

fn handle(config: &AppConfig, shown: &[Port], action: Action) {
    let lookup = |number: u16| shown.iter().find(|p| p.port == number);
    let result = match action {
        Action::Open(number) => lookup(number)
            .map(|port| actions::open(config, port))
            .unwrap_or(Ok(())),
        Action::CopyUrl(number) => {
            if let Some(port) = lookup(number) {
                let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
                clipboard.set_text(&config.url(port));
            }
            Ok(())
        }
        Action::Kill(number) | Action::ForceKill(number) => {
            let force = action == Action::ForceKill(number);
            // Look it up again so a stale menu never signals a recycled pid.
            actions::find(number).and_then(|port| {
                actions::kill(&port, force)?;
                notify::notify(
                    APP_NAME,
                    &format!("Stopped :{number}"),
                    &port.label(),
                    NOTIFY_TIMEOUT,
                );
                Ok(())
            })
        }
        Action::KillAll => {
            let stopped: Vec<String> = shown
                .iter()
                .filter_map(|shown| {
                    let port = actions::find(shown.port).ok()?;
                    match actions::kill(&port, false) {
                        Ok(()) => Some(format!(":{} {}", port.port, port.name())),
                        Err(err) => {
                            eprintln!("{err:#}");
                            None
                        }
                    }
                })
                .collect();
            if !stopped.is_empty() {
                notify::notify(
                    APP_NAME,
                    &format!("Stopped {} servers", stopped.len()),
                    &stopped.join("\n"),
                    NOTIFY_TIMEOUT,
                );
            }
            Ok(())
        }
    };
    if let Err(err) = result {
        eprintln!("{err:#}");
        notify::notify(APP_NAME, "typort", &format!("{err:#}"), NOTIFY_TIMEOUT);
    }
}

fn action(label: impl Into<String>, action: Action) -> MenuEntry<Action> {
    MenuEntry::Action {
        label: label.into(),
        enabled: true,
        action,
    }
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max - 1).collect();
    format!("{cut}…")
}

fn port_entries(config: &AppConfig, port: &Port) -> Vec<MenuEntry<Action>> {
    // Clicking the URL opens it.
    let mut entries = vec![
        action(config.url(port), Action::Open(port.port)),
        action("Copy URL", Action::CopyUrl(port.port)),
        MenuEntry::Separator,
    ];
    for process in &port.processes {
        entries.push(MenuEntry::Label(format!(
            "pid {}: {}",
            process.pid,
            shorten(&process.command_line(), MAX_COMMAND_CHARS)
        )));
    }
    if let Some(cwd) = port.processes.first().and_then(|p| p.cwd.as_ref()) {
        entries.push(MenuEntry::Label(format!("in {}", cwd.display())));
    }
    entries.push(MenuEntry::Label(port.reach()));
    let found = !port.processes.is_empty();
    entries.extend([
        MenuEntry::Separator,
        MenuEntry::Action {
            label: "Stop".into(),
            enabled: found,
            action: Action::Kill(port.port),
        },
        MenuEntry::Action {
            label: "Force kill".into(),
            enabled: found,
            action: Action::ForceKill(port.port),
        },
    ]);
    entries
}

fn spec(config: &AppConfig, ports: &[Port]) -> TraySpec<Action> {
    let summary = match ports.len() {
        0 => "No listening ports".to_string(),
        1 => "1 listening port".to_string(),
        n => format!("{n} listening ports"),
    };
    let mut menu = vec![MenuEntry::Label(summary.clone())];
    if !ports.is_empty() {
        menu.push(MenuEntry::Separator);
    }
    for port in ports {
        menu.push(MenuEntry::Submenu {
            label: match &port.title {
                Some(title) => format!(":{}  {title} · {}", port.port, port.label()),
                None => format!(":{}  {}", port.port, port.label()),
            },
            entries: port_entries(config, port),
        });
    }
    if !ports.is_empty() {
        menu.push(MenuEntry::Separator);
        menu.push(action("Stop all", Action::KillAll));
    }
    TraySpec {
        icon: if ports.is_empty() {
            IconState::Normal
        } else {
            IconState::Busy
        },
        title: format!("typort: {summary}"),
        menu,
        click_action: None,
    }
}
