//! Tray applet: flashes when a gesture fires, shows the last one, opens the window and can
//! pause tygesture.

use std::sync::mpsc;
use std::time::Duration;

use gtk::glib::{self, ControlFlow};
use ty_common::icons::Tool;
use ty_common::tray::{self, Applet, IconState, MenuEntry, TraySpec, TrayThread};

use crate::state::{Shared, Status};
use crate::APP;

const REFRESH: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Show,
    TogglePause,
    Quit,
}

pub fn start(shared: Shared) -> Option<TrayThread> {
    tray::spawn_gtk_thread(move || {
        let (actions_tx, actions_rx) = mpsc::channel();
        let mut applet = Applet::new(
            APP,
            &Tool::Gesture.tray_icons(),
            spec(&shared.lock().unwrap()),
            actions_tx,
        )?;

        let refresh = shared.clone();
        glib::timeout_add_local(REFRESH, move || {
            let show = {
                let mut status = refresh.lock().unwrap();
                applet.update(spec(&status));
                std::mem::take(&mut status.show_window)
            };
            if show {
                crate::gui::open(&refresh);
            }
            ControlFlow::Continue
        });

        tray::drain_actions(actions_rx, REFRESH, move |action| {
            match action {
                Action::Show => crate::gui::open(&shared),
                Action::TogglePause => {
                    let mut status = shared.lock().unwrap();
                    status.paused = !status.paused;
                }
                Action::Quit => std::process::exit(0),
            }
            ControlFlow::Continue
        });
        Ok(())
    })
}

fn spec(status: &Status) -> TraySpec<Action> {
    let lit = status.is_lit();
    let icon = if status.paused {
        IconState::Attention
    } else if lit {
        IconState::Busy
    } else {
        IconState::Normal
    };
    let touchpad = status.touchpad.as_deref().unwrap_or("no touchpad found");
    let last = match status.history.back() {
        Some(last) => format!("Last: {} ({} total)", last.gesture, status.fired),
        None => "No gestures yet".to_string(),
    };
    TraySpec {
        icon,
        title: if status.paused {
            format!("{APP} (paused)")
        } else {
            APP.to_string()
        },
        menu: vec![
            MenuEntry::Label(touchpad.to_string()),
            MenuEntry::Label(last),
            MenuEntry::Separator,
            MenuEntry::Action {
                label: "Show gestures…".to_string(),
                enabled: true,
                action: Action::Show,
            },
            MenuEntry::Action {
                label: if status.paused { "Resume" } else { "Pause" }.to_string(),
                enabled: true,
                action: Action::TogglePause,
            },
            MenuEntry::Action {
                label: "Quit".to_string(),
                enabled: true,
                action: Action::Quit,
            },
        ],
        click_action: Some(Action::TogglePause),
    }
}
