//! Tray applet: flashes when a gesture fires, shows the last one, opens the window and can
//! pause tygesture.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gtk::glib::{self, ControlFlow};
use ty_common::icons::Tool;
use ty_common::tray::{self, Applet, IconState, MenuEntry, TraySpec, TrayThread, Wakeups};

use crate::state::{Shared, Status};
use crate::APP;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Show,
    TogglePause,
    Quit,
}

type SharedApplet = Rc<RefCell<Applet<Action>>>;

/// `wakeups` fire whenever `shared` changes in a way the tray shows.
pub fn start(shared: Shared, wakeups: Wakeups) -> Option<TrayThread> {
    tray::spawn_gtk_thread(move || {
        let (actions_tx, actions_rx) = tray::channel();
        let applet: SharedApplet = Rc::new(RefCell::new(Applet::new(
            APP,
            &Tool::Gesture.tray_icons(),
            spec(&shared.lock().unwrap()),
            actions_tx,
        )?));

        let (on_wake_applet, on_wake_shared) = (applet.clone(), shared.clone());
        tray::on_wake(wakeups, move || {
            refresh(&on_wake_applet, &on_wake_shared);
            ControlFlow::Continue
        });

        let kept = applet.clone();
        tray::drain_actions(actions_rx, move |action| {
            match action {
                Action::Show => crate::gui::open(&shared),
                Action::TogglePause => {
                    let mut status = shared.lock().unwrap();
                    status.paused = !status.paused;
                }
                Action::Quit => std::process::exit(0),
            }
            refresh(&applet, &shared);
            ControlFlow::Continue
        });
        Ok(kept)
    })
}

/// Update the tray, open the window if asked, and come back when the flash ends.
fn refresh(applet: &SharedApplet, shared: &Shared) {
    let (show, flash_until) = {
        let mut status = shared.lock().unwrap();
        applet.borrow_mut().update(spec(&status));
        let lit = status.is_lit().then_some(status.flash_until).flatten();
        (std::mem::take(&mut status.show_window), lit)
    };
    if show {
        crate::gui::open(shared);
    }
    if let Some(until) = flash_until {
        let (applet, shared) = (applet.clone(), shared.clone());
        let wait = until.saturating_duration_since(Instant::now());
        glib::timeout_add_local_once(wait, move || refresh(&applet, &shared));
    }
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
