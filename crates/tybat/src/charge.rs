use std::path::PathBuf;
use std::process::Command;

use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use ty_common::tray::MenuEntry;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub percent: u8,
    /// Privileged helper, followed by fixed arguments; percentage is appended.
    pub command: Vec<String>,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (25..=100).contains(&self.percent),
            "charge_limit.percent must be 25–100"
        );
        ensure!(
            !self.command.is_empty() && !self.command[0].is_empty(),
            "charge_limit.command must name a helper"
        );
        Ok(())
    }
}

#[derive(Clone, PartialEq)]
pub enum Action {
    Set(u8),
    Reset,
    Reapply,
}

pub struct Controller {
    config: Config,
    selected: u8,
    applied: bool,
    error: Option<String>,
}

fn state_path() -> Result<PathBuf> {
    Ok(dirs::state_dir()
        .context("no XDG state directory")?
        .join("tybat/charge-limit"))
}

impl Controller {
    pub fn new(config: Config) -> Self {
        let saved = state_path()
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok());
        let values: Vec<u8> = saved
            .as_deref()
            .unwrap_or("")
            .split_whitespace()
            .filter_map(|v| v.parse().ok())
            .collect();
        // A changed declarative default invalidates an older tray override.
        let selected = if values.len() == 2
            && values[0] == config.percent
            && (25..=100).contains(&values[1])
        {
            values[1]
        } else {
            config.percent
        };
        let mut controller = Self {
            config,
            selected,
            applied: false,
            error: None,
        };
        controller.apply(selected, false);
        controller
    }

    fn apply(&mut self, percent: u8, save: bool) {
        let result = (|| -> Result<()> {
            let output = Command::new(&self.config.command[0])
                .args(&self.config.command[1..])
                .arg(percent.to_string())
                .output()
                .context("starting charge-limit helper")?;
            ensure!(
                output.status.success(),
                "charge-limit helper failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            self.selected = percent;
            self.applied = true;
            if save {
                let path = state_path()?;
                std::fs::create_dir_all(path.parent().context("missing state parent")?)?;
                let temp = path.with_extension("tmp");
                std::fs::write(&temp, format!("{} {}\n", self.config.percent, percent))?;
                std::fs::rename(temp, path).context("saving charge-limit override")?;
            }
            Ok(())
        })();
        self.error = result.err().map(|error| format!("{error:#}"));
        if let Some(error) = &self.error {
            eprintln!("tybat: {error}");
            ty_common::notify::notify(
                "tybat",
                "Charge limit could not be updated",
                error,
                std::time::Duration::from_secs(10),
            );
        }
    }

    pub fn handle(&mut self, action: Action) {
        if action == Action::Reapply {
            self.applied = false;
            self.apply(self.selected, false);
            return;
        }
        let percent = match action {
            Action::Set(percent) => percent,
            Action::Reset => self.config.percent,
            Action::Reapply => unreachable!(),
        };
        if (25..=100).contains(&percent) {
            self.apply(percent, true);
        }
    }

    pub fn menu(&self) -> MenuEntry<crate::devices::Action> {
        let mut entries = vec![MenuEntry::Label(format!(
            "Declared default: {}%",
            self.config.percent
        ))];
        if let Some(error) = &self.error {
            entries.push(MenuEntry::Label(error.clone()));
        }
        for percent in (25..=100).step_by(5) {
            entries.push(MenuEntry::Action {
                icon: None,
                label: format!(
                    "{}{}%",
                    if self.applied && percent == self.selected {
                        "✓ "
                    } else {
                        ""
                    },
                    percent
                ),
                enabled: true,
                action: crate::devices::Action::Charge(Action::Set(percent)),
            });
        }
        entries.push(MenuEntry::Action {
            label: "Reset to declared default".into(),
            icon: Some(ty_common::tray::MenuIcon::Reset),
            enabled: true,
            action: crate::devices::Action::Charge(Action::Reset),
        });
        MenuEntry::Submenu {
            icon: Some(ty_common::tray::MenuIcon::Battery),
            label: if self.applied {
                format!("Maximum charge: {}%", self.selected)
            } else {
                "Maximum charge: not applied".into()
            },
            entries,
        }
    }
}

/// logind emits false after both suspend and hibernate, even for brief sleeps.
pub fn watch_resume(sender: std::sync::mpsc::Sender<crate::devices::Action>) {
    std::thread::spawn(move || loop {
        let watch = || -> Result<()> {
            let conn = dbus::blocking::Connection::new_system()?;
            let rule = dbus::message::MatchRule::new_signal(
                "org.freedesktop.login1.Manager",
                "PrepareForSleep",
            )
            .with_sender("org.freedesktop.login1");
            let sender = sender.clone();
            conn.add_match(rule, move |(sleeping,): (bool,), _, _| {
                if !sleeping {
                    let _ = sender.send(crate::devices::Action::Charge(Action::Reapply));
                }
                true
            })?;
            loop {
                conn.process(std::time::Duration::from_secs(60))?;
            }
        };
        if let Err(error) = watch() {
            eprintln!("tybat: resume watcher: {error:#}");
        }
        std::thread::sleep(std::time::Duration::from_secs(10));
    });
}
