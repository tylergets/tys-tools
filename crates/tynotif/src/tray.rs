//! Notification history and do-not-disturb controls, using the shared tray menu.
use std::time::Duration;

use anyhow::Result;
use gtk::prelude::*;
use gtk::{gio, glib};
use ty_common::icons::Tool;
use ty_common::tray::{self, Applet, IconState, MenuEntry, TraySpec};

use crate::bus::{self, Shared};
use crate::model::State;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    TogglePause,
    Snooze(u64),
    History(u32),
    ClearHistory,
    DismissAll,
    Quit,
}

pub struct Tray {
    applet: Applet<Action>,
    revision: u64,
    remaining_minutes: Option<u64>,
}

impl Tray {
    pub fn new(state: Shared, connection: gio::DBusConnection) -> Result<Self> {
        let (tx, rx) = tray::channel();
        let applet = Applet::new(
            "tynotif",
            &Tool::Notif.tray_icons(),
            spec(&state.lock().unwrap()),
            tx,
        )?;
        tray::drain_actions(rx, move |action| {
            let mut state = state.lock().unwrap();
            match action {
                Action::TogglePause => {
                    let paused = !state.paused;
                    state.set_paused(paused);
                }
                Action::Snooze(minutes) => state.snooze(Duration::from_secs(minutes * 60)),
                Action::History(id) => {
                    let notification = state.history.iter().find(|n| n.id == id).cloned();
                    drop(state);
                    if let Some(notification) = notification {
                        show_history(&notification);
                    }
                }
                Action::ClearHistory => state.clear_history(),
                Action::DismissAll => bus::close_all(&connection, &mut state),
                Action::Quit => gtk::main_quit(),
            }
            glib::ControlFlow::Continue
        });
        Ok(Self {
            applet,
            revision: u64::MAX,
            remaining_minutes: None,
        })
    }

    pub fn refresh(&mut self, state: &State) {
        let minutes = remaining_minutes(state);
        if self.revision != state.revision || self.remaining_minutes != minutes {
            self.applet.update(spec(state));
            self.revision = state.revision;
            self.remaining_minutes = minutes;
        }
    }
}

fn remaining_minutes(state: &State) -> Option<u64> {
    state
        .snooze_remaining()
        .map(|remaining| remaining.as_secs().div_ceil(60))
}

fn title(text: &str) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut short: String = clean.chars().take(80).collect();
    if clean.chars().count() > 80 {
        short.push('…');
    }
    short
}

fn spec(state: &State) -> TraySpec<Action> {
    let action = |label: &str, action| MenuEntry::Action {
        label: label.into(),
        enabled: true,
        action,
    };
    let status = match (state.paused, remaining_minutes(state)) {
        (true, Some(minutes)) => format!("Sleeping · {minutes} min remaining"),
        (true, None) => "Paused until resumed".into(),
        _ => "Notifications enabled".into(),
    };
    let history = if state.history.is_empty() {
        vec![MenuEntry::Label("No notification history yet".into())]
    } else {
        state
            .history
            .iter()
            .map(|n| MenuEntry::Action {
                label: title(&format!("{} — {}", n.app, n.summary)),
                enabled: true,
                action: Action::History(n.id),
            })
            .collect()
    };
    TraySpec {
        icon: if state.paused {
            IconState::Busy
        } else {
            IconState::Normal
        },
        title: format!("tynotif — {status}"),
        menu: vec![
            MenuEntry::Label(status),
            action(
                if state.paused {
                    "Resume notifications"
                } else {
                    "Pause until resumed"
                },
                Action::TogglePause,
            ),
            MenuEntry::Submenu {
                label: "Sleep notifications".into(),
                entries: [15, 30, 60, 120]
                    .into_iter()
                    .map(|minutes| {
                        action(&format!("For {minutes} minutes"), Action::Snooze(minutes))
                    })
                    .collect(),
            },
            MenuEntry::Separator,
            MenuEntry::Submenu {
                label: format!("History ({})", state.history.len()),
                entries: history,
            },
            MenuEntry::Action {
                label: "Clear history".into(),
                enabled: !state.history.is_empty(),
                action: Action::ClearHistory,
            },
            MenuEntry::Action {
                label: format!("Dismiss all ({})", state.active.len()),
                enabled: !state.active.is_empty(),
                action: Action::DismissAll,
            },
            MenuEntry::Separator,
            action("Quit", Action::Quit),
        ],
        click_action: Some(Action::TogglePause),
    }
}

fn show_history(notification: &crate::model::Notification) {
    let window = ty_common::overlay::dialog_window("tynotif-history", "Notification history");
    window.set_default_size(520, 360);
    window.set_resizable(true);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_border_width(18);
    let details = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let heading = gtk::Label::new(Some(&format!(
        "{} — {}",
        notification.app, notification.summary
    )));
    heading.set_xalign(0.0);
    heading.set_line_wrap(true);
    heading.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);
    heading.set_max_width_chars(60);
    heading.set_selectable(true);
    details.pack_start(&heading, false, false, 0);
    let body = gtk::Label::new(Some(&notification.body));
    body.set_selectable(true);
    body.set_line_wrap(true);
    body.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);
    body.set_max_width_chars(60);
    body.set_xalign(0.0);
    body.set_yalign(0.0);
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    details.pack_start(&body, false, false, 0);
    scroll.add(&details);
    content.pack_start(&scroll, true, true, 0);
    let close = gtk::Button::with_label("Close");
    let weak = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    });
    content.pack_end(&close, false, false, 0);
    window.connect_key_press_event(|window, event| {
        if event.keyval() == gtk::gdk::keys::constants::Escape {
            window.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add(&content);
    window.show_all();
    window.present();
}
