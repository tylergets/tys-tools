//! Tray applet (StatusNotifierItem, via `ty_common::tray`). Each capture runs `tyshot gui` or
//! `tyshot full` as a child process, so the tray only launches them and stays out of the
//! way of the overlay's input grab.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use gtk::glib::ControlFlow;
use ty_common::icons::Tool;
use ty_common::tray::{drain_actions, Applet, IconState, MenuEntry, TraySpec};

use crate::config::AppConfig;

const ID: &str = "tyshot";
/// Gives the tray menu time to disappear from the screen before the snapshot is taken.
const MENU_CLOSE_DELAY: &str = "0.25";
const ACTION_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Tray menu and click actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    CaptureRegion,
    CaptureScreen,
    OpenFolder,
    Quit,
}

/// The (static) tray: middle-click (if the tray host forwards it) captures a region.
fn spec() -> TraySpec<TrayAction> {
    let action = |label: &str, action| MenuEntry::Action {
        label: label.to_string(),
        enabled: true,
        action,
    };
    TraySpec {
        icon: IconState::Normal,
        title: ID.to_string(),
        menu: vec![
            action("Capture Region", TrayAction::CaptureRegion),
            action("Capture Screen", TrayAction::CaptureScreen),
            MenuEntry::Separator,
            action("Open Screenshots Folder", TrayAction::OpenFolder),
            MenuEntry::Separator,
            action("Quit", TrayAction::Quit),
        ],
        click_action: Some(TrayAction::CaptureRegion),
    }
}

/// Run the tray until Quit is chosen. `config_path` is forwarded to the capture commands.
pub fn run(config: AppConfig, config_path: Option<PathBuf>) -> Result<()> {
    gtk::init().context("failed to initialize GTK")?;
    let exe = env::current_exe().context("failed to find the tyshot executable")?;
    let launcher = Launcher { exe, config_path };

    let (tx, rx) = mpsc::channel();
    // Kept alive until gtk::main returns.
    let _applet = Applet::new(ID, &Tool::Shot.tray_icons(), spec(), tx)?;

    let save = config.save;
    drain_actions(rx, ACTION_POLL_INTERVAL, move |action| {
        match action {
            TrayAction::CaptureRegion => launcher.spawn(&["gui", "--delay", MENU_CLOSE_DELAY]),
            TrayAction::CaptureScreen => launcher.spawn(&["full", "--delay", MENU_CLOSE_DELAY]),
            TrayAction::OpenFolder => match save.resolved_output_dir() {
                Ok(dir) => open_folder(&dir),
                Err(err) => eprintln!("error: {err:#}"),
            },
            TrayAction::Quit => {
                gtk::main_quit();
                return ControlFlow::Break;
            }
        }
        ControlFlow::Continue
    });

    gtk::main();
    Ok(())
}

struct Launcher {
    exe: PathBuf,
    config_path: Option<PathBuf>,
}

impl Launcher {
    fn spawn(&self, args: &[&str]) {
        let mut command = Command::new(&self.exe);
        if let Some(path) = &self.config_path {
            command.arg("--config").arg(path);
        }
        match command.args(args).spawn() {
            // Reap the child without blocking the GTK loop.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(err) => eprintln!("error: failed to start tyshot {}: {err}", args[0]),
        }
    }
}

fn open_folder(dir: &Path) {
    if let Err(err) = fs::create_dir_all(dir) {
        eprintln!("error: failed to create {}: {err}", dir.display());
        return;
    }
    match Command::new("xdg-open").arg(dir).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => eprintln!("error: failed to launch xdg-open: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{spec, TrayAction};
    use ty_common::tray::MenuEntry;

    #[test]
    fn middle_click_captures_a_region_from_the_menu() {
        let spec = spec();
        let click = spec.click_action.expect("click action");
        assert_eq!(click, TrayAction::CaptureRegion);
        // The applet can only target a top-level menu item.
        assert!(spec
            .menu
            .iter()
            .any(|entry| matches!(entry, MenuEntry::Action { action, .. } if *action == click)));
    }
}
