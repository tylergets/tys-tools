mod devices;

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand};
use gtk::{cairo, gdk, glib};
use serde::Deserialize;
use ty_common::{appearance, config, lock::InstanceLock, notify, overlay::Overlay};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Monitor batteries in the graphical session.
    Run,
    /// Preview a critical popup without performing a power action.
    Preview,
    /// Print laptop and connected device battery levels.
    Status,
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Action {
    Suspend,
    Hibernate,
    None,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::Suspend => "Suspend",
            Self::Hibernate => "Hibernate",
            Self::None => "Reminder",
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    warning_percent: u8,
    critical_percent: u8,
    countdown_seconds: u32,
    snooze_seconds: u32,
    poll_seconds: u32,
    action: Action,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            warning_percent: 20,
            critical_percent: 5,
            countdown_seconds: 60,
            snooze_seconds: 60,
            poll_seconds: 5,
            action: Action::Suspend,
        }
    }
}

impl Config {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.critical_percent > 0
                && self.critical_percent < self.warning_percent
                && self.warning_percent <= 100,
            "require 0 < critical_percent < warning_percent <= 100"
        );
        ensure!(
            self.countdown_seconds > 0 && self.snooze_seconds > 0 && self.poll_seconds > 0,
            "countdown_seconds, snooze_seconds and poll_seconds must be positive"
        );
        Ok(())
    }
}

/// Use the lowest discharging system battery, never a mouse/headset battery.
/// An online external supply cancels the countdown even before status changes.
fn battery() -> Result<Option<u8>> {
    let mut lowest = None;
    let mut online = false;
    for entry in fs::read_dir("/sys/class/power_supply")? {
        let path = entry?.path();
        let read = |name: &str| fs::read_to_string(path.join(name)).map(|s| s.trim().to_owned());
        if read("scope").is_ok_and(|s| s == "Device") {
            continue;
        }
        let kind = read("type")?;
        if kind != "Battery" {
            online |= read("online").is_ok_and(|s| s == "1");
            continue;
        }
        if read("present").is_ok_and(|s| s == "0") {
            continue;
        }
        if read("status")? != "Discharging" {
            continue;
        }
        let capacity: u8 = read("capacity")?
            .parse()
            .context("invalid battery capacity")?;
        ensure!(capacity <= 100, "battery capacity exceeds 100");
        lowest = Some(lowest.map_or(capacity, |old: u8| old.min(capacity)));
    }
    Ok(if online { None } else { lowest })
}

struct Popup {
    overlay: Overlay,
    remaining: Rc<Cell<u32>>,
    percent: Rc<Cell<u8>>,
    snoozed: Rc<Cell<bool>>,
    deadline: Instant,
}

impl Drop for Popup {
    fn drop(&mut self) {
        self.overlay.close();
    }
}

fn popup(cfg: &Config, percent: u8, preview: bool) -> Result<Popup> {
    let overlay = Overlay::new("tybat")?;
    let (cx, cy) = overlay.pointer_monitor_center();
    let remaining = Rc::new(Cell::new(cfg.countdown_seconds));
    let level = Rc::new(Cell::new(percent));
    let snoozed = Rc::new(Cell::new(false));
    let seconds = remaining.clone();
    let battery = level.clone();
    let action = cfg.action;
    let snooze_seconds = cfg.snooze_seconds;
    overlay.connect_draw(move |cr| {
        cr.set_source_rgba(0.02, 0.03, 0.05, 0.90);
        let _ = cr.paint();
        cr.set_source_rgb(0.12, 0.14, 0.18);
        cr.rectangle(cx - 280.0, cy - 155.0, 560.0, 310.0);
        let _ = cr.fill();
        let text = |text: &str, y: f64, size: f64| {
            appearance::select_font(cr, cairo::FontWeight::Bold);
            cr.set_font_size(size);
            if let Ok(extents) = cr.text_extents(text) {
                cr.move_to(cx - extents.width() / 2.0 - extents.x_bearing(), y);
                let _ = cr.show_text(text);
            }
        };
        cr.set_source_rgb(1.0, 0.42, 0.38);
        text(
            &format!("Battery critical · {}%", battery.get()),
            cy - 94.0,
            28.0,
        );
        cr.set_source_rgb(0.95, 0.96, 0.98);
        text(
            &format!("{} in {}s", action.label(), seconds.get()),
            cy - 25.0,
            36.0,
        );
        text("Plug in your charger to cancel", cy + 18.0, 20.0);
        cr.set_source_rgb(0.62, 0.72, 0.90);
        text(
            &format!("[ Snooze {snooze_seconds}s · Esc ]"),
            cy + 82.0,
            22.0,
        );
        if preview {
            text("Preview — no power action", cy + 126.0, 16.0);
        }
    });
    let dismissed = snoozed.clone();
    overlay.connect_key_press(move |overlay, event| {
        if event.keyval() == gdk::keys::constants::Escape {
            dismissed.set(true);
            overlay.close();
        }
    });
    let dismissed = snoozed.clone();
    overlay.connect_button_press(move |overlay, event, (x, y)| {
        if event.button() == 1 && (x - cx).abs() < 230.0 && (cy + 48.0..cy + 100.0).contains(&y) {
            dismissed.set(true);
            overlay.close();
        }
    });
    overlay.show(gdk::CursorType::LeftPtr, || {
        eprintln!("tybat: unable to grab input; critical popup remains visible");
    });
    Ok(Popup {
        overlay,
        remaining,
        percent: level,
        snoozed,
        deadline: Instant::now() + Duration::from_secs(cfg.countdown_seconds.into()),
    })
}

#[derive(Default)]
struct Monitor {
    popup: Option<Popup>,
    snooze_until: Option<Instant>,
    last_poll: Option<Instant>,
    warned: bool,
    action: Option<std::process::Child>,
}

impl Monitor {
    fn tick(&mut self, cfg: &Config) -> Result<()> {
        let now = Instant::now();
        if let Some(child) = &mut self.action {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    notify::notify(
                        "tybat",
                        "Battery power action failed",
                        "Plug in your charger.",
                        Duration::from_secs(10),
                    );
                    eprintln!("tybat: power action failed: {status}");
                }
                self.action = None;
            } else {
                return Ok(());
            }
        }
        // Read every second while counting down, including immediately before acting.
        if self.popup.is_none()
            && self.last_poll.is_some_and(|t| {
                now.duration_since(t) < Duration::from_secs(cfg.poll_seconds.into())
            })
        {
            return Ok(());
        }
        self.last_poll = Some(now);
        let level = match battery() {
            Ok(level) => level,
            Err(error) => {
                // Never perform a power action on stale or unreadable battery data.
                self.popup = None;
                return Err(error);
            }
        };
        let Some(level) = level else {
            self.popup = None;
            self.snooze_until = None;
            self.warned = false;
            return Ok(());
        };
        if level > cfg.warning_percent.saturating_add(2) {
            self.warned = false;
        }
        if level > cfg.critical_percent {
            self.popup = None;
            self.snooze_until = None;
        }
        if level <= cfg.warning_percent && !self.warned {
            notify::notify(
                "tybat",
                "Battery low",
                &format!("{level}% remaining. Plug in your charger."),
                Duration::from_secs(10),
            );
            self.warned = true;
        }
        if let Some(popup) = &self.popup {
            if popup.snoozed.get() {
                self.popup = None;
                self.snooze_until = Some(now + Duration::from_secs(cfg.snooze_seconds.into()));
                return Ok(());
            }
            popup.percent.set(level);
            let left = popup
                .deadline
                .saturating_duration_since(now)
                .as_secs_f64()
                .ceil() as u32;
            popup.remaining.set(left);
            popup.overlay.queue_draw();
            if left == 0 {
                self.popup = None;
                self.snooze_until = Some(now + Duration::from_secs(cfg.snooze_seconds.into()));
                let arg = match cfg.action {
                    Action::Suspend => "suspend",
                    Action::Hibernate => "hibernate",
                    Action::None => return Ok(()),
                };
                self.action = Some(
                    std::process::Command::new("systemctl")
                        .arg(arg)
                        .spawn()
                        .context("starting battery power action")?,
                );
            }
        } else if level <= cfg.critical_percent
            && self.snooze_until.is_none_or(|until| now >= until)
        {
            self.popup = Some(popup(cfg, level, false)?);
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg: Config = config::load("tybat", cli.config)?.config;
    cfg.validate()?;
    let command = cli.command.unwrap_or(Command::Run);
    if matches!(command, Command::Status) {
        let devices = devices::scan()?;
        for entry in devices::spec(&Ok(devices)).0.menu {
            if let ty_common::tray::MenuEntry::Label(line) = entry {
                println!("{line}");
            }
        }
        return Ok(());
    }
    gtk::init()?;
    appearance::apply_gtk();
    if matches!(command, Command::Preview) {
        let popup = popup(&cfg, cfg.critical_percent, true)?;
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let left = popup
                .deadline
                .saturating_duration_since(Instant::now())
                .as_secs_f64()
                .ceil() as u32;
            popup.remaining.set(left);
            popup.overlay.queue_draw();
            if popup.snoozed.get() || left == 0 {
                popup.overlay.close();
                gtk::main_quit();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        gtk::main();
        return Ok(());
    }
    let _lock = InstanceLock::acquire("tybat")?;
    ty_common::lock::serve(_lock.listener(), |_| "running".into())?;
    devices::start()?;
    let monitor = RefCell::new(Monitor::default());
    if let Err(error) = monitor.borrow_mut().tick(&cfg) {
        eprintln!("tybat: {error:#}");
    }
    glib::timeout_add_local(Duration::from_secs(1), move || {
        if let Err(error) = monitor.borrow_mut().tick(&cfg) {
            eprintln!("tybat: {error:#}");
        }
        glib::ControlFlow::Continue
    });
    gtk::main();
    Ok(())
}
