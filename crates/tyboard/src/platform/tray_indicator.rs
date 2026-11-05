//! The tray applet: a GTK thread that polls the clipboard, records history and shows it in
//! an AppIndicator menu (see `ty_common::tray`).

use std::cell::RefCell;
use std::process;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gtk::glib::{self, ControlFlow};
use ty_common::icons::Tool;
use ty_common::tray::{drain_actions, Applet, IconState, MenuEntry, TraySpec};

use crate::clipboard::backend::{
    menu_label, set_clipboard_value, GtkClipboardBackend, WlClipboardBackend,
};
use crate::clipboard::poller::ClipboardPoller;
use crate::clipboard::{ClipboardEntry, SharedClipboardState};
use crate::config::{ClipboardBackend, ClipboardConfig};
use crate::core::active_window::provider_from_config;
use crate::core::capture_control::{is_capture_paused, set_capture_paused};

const APP_ID: &str = "tyboard";
const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(750);
const ACTION_POLL_INTERVAL: Duration = Duration::from_millis(50);
const NOTIFICATION_TIMEOUT: Duration = Duration::from_millis(1000);
const MENU_LABEL_CHAR_LIMIT: usize = 70;

pub use ty_common::tray::TrayThread as TrayIndicator;

/// Tray menu actions. `CopyEntry` is an index into the history snapshot (newest first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    TogglePause,
    CopyEntry(usize),
    ClearHistory,
    Quit,
}

pub fn start(
    shared_state: SharedClipboardState,
    clipboard_config: ClipboardConfig,
) -> Option<TrayIndicator> {
    ty_common::tray::spawn_gtk_thread(move || {
        let provider = provider_from_config(&clipboard_config.active_window.backend);
        let backend: Box<dyn crate::clipboard::backend::ClipboardBackend> =
            match clipboard_config.backend {
                ClipboardBackend::Gtk => Box::new(gtk_backend(provider)),
                ClipboardBackend::WlClipboard => Box::new(WlClipboardBackend::new(provider)?),
                ClipboardBackend::Auto if ty_common::session::is_wayland() => {
                    Box::new(WlClipboardBackend::new(provider)?)
                }
                ClipboardBackend::Auto => Box::new(gtk_backend(provider)),
            };
        let mut poller =
            ClipboardPoller::new(backend, clipboard_config.active_window.blacklist.clone());
        if let Some(entry) = poller.poll_once() {
            if let Err(err) = shared_state.record_entry(entry) {
                eprintln!("failed to seed clipboard history: {err}");
            }
        }

        let paused = is_capture_paused().unwrap_or_else(|err| {
            eprintln!("warning: failed to read capture pause state: {err}");
            false
        });
        let (actions_tx, actions_rx) = mpsc::channel();
        let spec = tray_spec(paused, &shared_state.history_snapshot());
        let applet = Applet::new(APP_ID, &Tool::Board.tray_icons(), spec, actions_tx)?;
        let tray = Rc::new(RefCell::new(Tray {
            applet,
            state: shared_state,
            paused,
        }));

        let tray_for_actions = tray.clone();
        drain_actions(actions_rx, ACTION_POLL_INTERVAL, move |action| {
            handle_action(&tray_for_actions, action);
            ControlFlow::Continue
        });

        let tray_for_poll = tray.clone();
        glib::timeout_add_local(CLIPBOARD_POLL_INTERVAL, move || {
            let entry = poller.poll_once();
            let mut tray = tray_for_poll.borrow_mut();
            let paused = tray.read_paused();
            let mut changed = paused != tray.paused;
            tray.paused = paused;

            if let Some(entry) = entry.filter(|_| !paused) {
                let body = match entry {
                    ClipboardEntry::Text { .. } => "text copied to clipboard",
                    ClipboardEntry::Image { .. } => "image copied to clipboard",
                };
                let recorded = tray.state.record_entry(entry).unwrap_or_else(|err| {
                    eprintln!("failed to record clipboard history: {err}");
                    false
                });
                if recorded {
                    println!("clipboard event: {body}");
                    ty_common::notify::notify(APP_ID, APP_ID, body, NOTIFICATION_TIMEOUT);
                    changed = true;
                }
            }

            if changed {
                tray.refresh();
            }
            ControlFlow::Continue
        });

        Ok(tray)
    })
}

pub fn tray_spec(paused: bool, history: &[ClipboardEntry]) -> TraySpec<TrayAction> {
    let history_entries = if history.is_empty() {
        vec![MenuEntry::Label("No clipboard history yet".to_string())]
    } else {
        history
            .iter()
            .enumerate()
            .map(|(index, entry)| MenuEntry::Action {
                label: menu_label(entry, MENU_LABEL_CHAR_LIMIT),
                enabled: true,
                action: TrayAction::CopyEntry(index),
            })
            .collect()
    };
    let action = |label: &str, action| MenuEntry::Action {
        label: label.to_string(),
        enabled: true,
        action,
    };

    TraySpec {
        icon: IconState::Normal,
        title: APP_ID.to_string(),
        menu: vec![
            MenuEntry::Label(if paused { "Paused" } else { "Running" }.to_string()),
            action(
                if paused {
                    "Resume Capture"
                } else {
                    "Pause Capture"
                },
                TrayAction::TogglePause,
            ),
            MenuEntry::Separator,
            MenuEntry::Submenu {
                label: "History".to_string(),
                entries: history_entries,
            },
            action("Clear History", TrayAction::ClearHistory),
            MenuEntry::Separator,
            action("Quit", TrayAction::Quit),
        ],
        click_action: None,
    }
}

fn gtk_backend(
    provider: Box<dyn crate::core::active_window::ActiveWindowProvider>,
) -> GtkClipboardBackend {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    GtkClipboardBackend::new(&clipboard, provider)
}

/// GTK-thread state. Never borrowed across a call that can spin a nested main loop
/// (clipboard reads and `store()`), since the poll timer and action handler share it.
struct Tray {
    applet: Applet<TrayAction>,
    state: SharedClipboardState,
    paused: bool,
}

impl Tray {
    fn refresh(&mut self) {
        let spec = tray_spec(self.paused, &self.state.history_snapshot());
        self.applet.update(spec);
    }

    fn read_paused(&self) -> bool {
        is_capture_paused().unwrap_or_else(|err| {
            eprintln!("warning: failed to read capture pause state: {err}");
            self.paused
        })
    }
}

fn handle_action(tray: &RefCell<Tray>, action: TrayAction) {
    match action {
        TrayAction::TogglePause => {
            let mut tray = tray.borrow_mut();
            let next = !tray.paused;
            if let Err(err) = set_capture_paused(next) {
                eprintln!("failed to update capture pause state: {err}");
                return;
            }
            tray.paused = next;
            tray.refresh();
        }
        TrayAction::CopyEntry(index) => {
            let entry = tray.borrow().state.history_snapshot().get(index).cloned();
            // The next poll picks the new value up and moves it to the top of the history.
            if let Some(entry) = entry {
                if let Err(err) = set_clipboard_value(&entry) {
                    eprintln!("failed to set the clipboard: {err:#}");
                }
            }
        }
        TrayAction::ClearHistory => {
            let mut tray = tray.borrow_mut();
            if let Err(err) = tray.state.clear_history() {
                eprintln!("failed to clear clipboard history: {err}");
                return;
            }
            tray.refresh();
        }
        TrayAction::Quit => process::exit(0),
    }
}

#[cfg(test)]
mod tests {
    use super::{tray_spec, TrayAction};
    use crate::clipboard::ClipboardEntry;
    use ty_common::tray::MenuEntry;

    fn text(value: &str) -> ClipboardEntry {
        ClipboardEntry::Text {
            value: value.to_string(),
            source_window: None,
        }
    }

    fn label(entry: &MenuEntry<TrayAction>) -> &str {
        match entry {
            MenuEntry::Label(label)
            | MenuEntry::Action { label, .. }
            | MenuEntry::Submenu { label, .. } => label,
            MenuEntry::Separator => "",
        }
    }

    #[test]
    fn shows_pause_state() {
        let running = tray_spec(false, &[]);
        assert_eq!(label(&running.menu[0]), "Running");
        assert_eq!(label(&running.menu[1]), "Pause Capture");
        let paused = tray_spec(true, &[]);
        assert_eq!(label(&paused.menu[0]), "Paused");
        assert_eq!(label(&paused.menu[1]), "Resume Capture");
    }

    #[test]
    fn lists_history_with_truncated_labels() {
        let long = "x".repeat(80);
        let spec = tray_spec(false, &[text("a\nb"), text(&long)]);
        let MenuEntry::Submenu { entries, .. } = &spec.menu[3] else {
            panic!("expected history submenu");
        };
        assert_eq!(
            entries[0],
            MenuEntry::Action {
                label: "a\\nb".to_string(),
                enabled: true,
                action: TrayAction::CopyEntry(0),
            }
        );
        assert_eq!(label(&entries[1]), format!("{}...", "x".repeat(70)));
    }

    #[test]
    fn empty_history_shows_placeholder() {
        let spec = tray_spec(false, &[]);
        let MenuEntry::Submenu { entries, .. } = &spec.menu[3] else {
            panic!("expected history submenu");
        };
        assert_eq!(
            entries,
            &[MenuEntry::Label("No clipboard history yet".to_string())]
        );
    }
}
