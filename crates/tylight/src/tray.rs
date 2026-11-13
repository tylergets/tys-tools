//! Tray applet: current state, toggle, timed disable, temperature and monitor brightness.

use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::thread;

use gtk::glib::ControlFlow;
use ty_common::icons::Tool;
use ty_common::tray::{self, Applet, IconState, MenuEntry, TraySpec, TrayThread, Wakeups};

use crate::config::APP_NAME;
use crate::daemon::{Msg, Shared};
use crate::monitors::Change;
use crate::protocol::{Command, Status};

const TEMPERATURES: [u32; 7] = [6500, 5500, 4500, 3500, 3100, 2700, 2000];
const BRIGHTNESS_LEVELS: [u8; 5] = [100, 75, 50, 25, 10];

/// `wakeups` fire whenever the shared status changes.
pub fn start(tx: Sender<Msg>, shared: Shared, wakeups: Wakeups) -> Option<TrayThread> {
    let step = i32::from(shared.config.monitor_brightness.step);
    let presets: Vec<String> = shared.config.presets.keys().cloned().collect();
    tray::spawn_gtk_thread(move || {
        let (actions_tx, actions_rx) = tray::channel::<Command>();
        let mut applet = Applet::new(
            APP_NAME,
            &Tool::Light.tray_icons(),
            spec(&shared.status(), step, &presets),
            actions_tx,
        )?;

        shared.gui.store(true, Ordering::Relaxed);

        let refresh_shared = shared.clone();
        tray::on_wake(wakeups, move || {
            applet.update(spec(&refresh_shared.status(), step, &presets));
            if refresh_shared.show_graph.swap(false, Ordering::Relaxed) {
                crate::graph::open(&refresh_shared);
            }
            ControlFlow::Continue
        });

        tray::drain_actions(actions_rx, move |command| {
            match command {
                Command::Brightness(change) => {
                    let shared = shared.clone();
                    thread::spawn(move || {
                        if let Err(err) = shared.change_brightness(change) {
                            eprintln!("brightness: {err:#}");
                        }
                    });
                }
                Command::AutoBrightness(on) => {
                    if let Err(err) = shared.set_auto_brightness(on) {
                        eprintln!("auto brightness: {err:#}");
                    }
                }
                Command::ShowGraph => crate::graph::open(&shared),
                command => {
                    if tx.send(Msg::Command(command, None)).is_err() {
                        return ControlFlow::Break;
                    }
                }
            }
            ControlFlow::Continue
        });
        Ok(())
    })
}

fn action(label: impl Into<String>, action: Command) -> MenuEntry<Command> {
    MenuEntry::Action {
        label: label.into(),
        enabled: true,
        action,
    }
}

fn spec(status: &Status, step: i32, presets: &[String]) -> TraySpec<Command> {
    let mut menu = vec![MenuEntry::Label(status.summary())];
    menu.push(MenuEntry::Label(
        match (&status.location, &status.location_source) {
            (Some(l), Some(source)) => {
                format!("Location: {:.2}, {:.2} ({source})", l.latitude, l.longitude)
            }
            _ => "Location: waiting for GeoClue".into(),
        },
    ));
    if let Some(lux) = status.ambient_lux {
        menu.push(MenuEntry::Label(format!("Ambient light: {lux:.0} lux")));
    }
    menu.push(MenuEntry::Separator);

    menu.push(action(
        if status.enabled { "Disable" } else { "Enable" },
        Command::Toggle,
    ));
    menu.push(MenuEntry::Submenu {
        label: "Disable for".into(),
        entries: vec![
            action("30 minutes", Command::Disable(Some(30 * 60))),
            action("1 hour", Command::Disable(Some(3600))),
            action("2 hours", Command::Disable(Some(2 * 3600))),
            MenuEntry::Action {
                label: "Until morning".into(),
                enabled: status.location.is_some(),
                action: Command::DisableUntilMorning,
            },
        ],
    });

    if !presets.is_empty() {
        let mut entries = vec![action(
            if status.preset.is_none() && status.manual.is_none() {
                "• Follow the sun"
            } else {
                "Follow the sun"
            },
            Command::Auto,
        )];
        entries.extend(presets.iter().map(|name| {
            let mark = if status.preset.as_ref() == Some(name) {
                "• "
            } else {
                ""
            };
            action(format!("{mark}{name}"), Command::Preset(name.clone()))
        }));
        menu.push(MenuEntry::Submenu {
            label: "Presets".into(),
            entries,
        });
    }

    let mut temperatures = vec![action(
        if status.manual.is_none() && status.preset.is_none() {
            "• Automatic"
        } else {
            "Automatic"
        },
        Command::Auto,
    )];
    temperatures.extend(TEMPERATURES.map(|kelvin| {
        let mark = if status.manual == Some(kelvin) {
            "• "
        } else {
            ""
        };
        action(format!("{mark}{kelvin}K"), Command::Set(kelvin))
    }));
    menu.push(MenuEntry::Submenu {
        label: "Temperature".into(),
        entries: temperatures,
    });

    if let Some(percent) = status.monitor_brightness {
        let mut entries = vec![
            action("Brighter", Command::Brightness(Change::By(step))),
            action("Dimmer", Command::Brightness(Change::By(-step))),
            MenuEntry::Separator,
        ];
        entries.extend(
            BRIGHTNESS_LEVELS
                .map(|level| action(format!("{level}%"), Command::Brightness(Change::Set(level)))),
        );
        let auto = status.auto_brightness == Some(true);
        menu.push(MenuEntry::Submenu {
            label: format!(
                "Monitor brightness ({percent}%{})",
                if auto { ", auto" } else { "" }
            ),
            entries,
        });
    }
    if let Some(auto) = status.auto_brightness {
        menu.push(action(
            if auto {
                "✓ Auto brightness"
            } else {
                "Auto brightness"
            },
            Command::AutoBrightness(None),
        ));
    }
    menu.push(MenuEntry::Separator);
    menu.push(action("Show graph…", Command::ShowGraph));

    let warm = status.enabled && status.suspended_by.is_empty() && status.temperature < 6400;
    TraySpec {
        icon: if warm {
            IconState::Busy
        } else {
            IconState::Normal
        },
        title: format!("tylight: {}", status.summary()),
        menu,
        click_action: Some(Command::Toggle),
    }
}
