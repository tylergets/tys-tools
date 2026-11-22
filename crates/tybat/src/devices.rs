use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::Result;
use dbus::arg::PropMap;
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
use dbus::blocking::Connection;
use ty_common::tray::{IconState, MenuEntry, TraySpec};

const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, PartialEq)]
pub enum Action {
    Charge(crate::charge::Action),
    Profile(String),
}

#[derive(Clone)]
pub struct Device {
    name: String,
    native: String,
    percent: Option<f64>,
    state: String,
    laptop: bool,
    drain_watts: Option<f64>,
    seconds_left: Option<f64>,
    percent_per_hour: Option<f64>,
}

fn text(props: &PropMap, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| v.0.as_str())
        .unwrap_or("")
        .to_owned()
}

fn number(props: &PropMap, key: &str) -> u64 {
    props.get(key).and_then(|v| v.0.as_u64()).unwrap_or(0)
}

fn positive(props: &PropMap, key: &str) -> Option<f64> {
    props
        .get(key)
        .and_then(|v| v.0.as_f64())
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn remaining_time(seconds: f64) -> String {
    let minutes = (seconds / 60.0).ceil() as u64;
    if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

#[derive(Default)]
struct History {
    samples: VecDeque<(Duration, f64)>,
}

impl History {
    fn observe(&mut self, now: Duration, percent: f64) -> Option<f64> {
        if self.samples.back().is_some_and(|&(time, previous)| {
            now < time || now.saturating_sub(time) > Duration::from_secs(60) || percent > previous
        }) {
            self.samples.clear();
        }
        self.samples.push_back((now, percent));
        while self
            .samples
            .front()
            .is_some_and(|&(time, _)| now.saturating_sub(time) > Duration::from_secs(30 * 60))
        {
            self.samples.pop_front();
        }
        let &(start, initial) = self.samples.front()?;
        let elapsed = now.saturating_sub(start).as_secs_f64();
        let drop = initial - percent;
        (elapsed >= 60.0 && drop >= 1.0).then(|| drop * 3600.0 / elapsed)
    }
}

#[derive(Default)]
struct Tracker(HashMap<String, History>);

impl Tracker {
    fn update(&mut self, result: &mut Result<Vec<Device>>, now: Duration) {
        let Ok(devices) = result else {
            self.0.clear();
            return;
        };
        self.0.retain(|id, _| {
            devices
                .iter()
                .any(|d| &d.native == id && d.state == "discharging" && d.percent.is_some())
        });
        for device in devices {
            device.percent_per_hour = None;
            device.seconds_left = None;
            if device.state != "discharging" {
                continue;
            }
            if let Some(percent) = device.percent {
                let rate = self
                    .0
                    .entry(device.native.clone())
                    .or_default()
                    .observe(now, percent);
                device.percent_per_hour = rate;
                device.seconds_left = rate.map(|rate| percent / rate * 3600.0);
            }
        }
    }
}

pub fn scan() -> Result<Vec<Device>> {
    let conn = Connection::new_system()?;
    let proxy = conn.with_proxy("org.freedesktop.UPower", "/org/freedesktop/UPower", TIMEOUT);
    let (paths,): (Vec<dbus::Path<'static>>,) =
        proxy.method_call("org.freedesktop.UPower", "EnumerateDevices", ())?;
    let mut devices = Vec::new();
    for path in paths {
        let proxy = conn.with_proxy("org.freedesktop.UPower", path.clone(), TIMEOUT);
        let Ok(props) = proxy.get_all("org.freedesktop.UPower.Device") else {
            continue;
        };
        let kind = number(&props, "Type");
        if kind == 1 || number(&props, "IsPresent") == 0 {
            continue;
        }
        let model = text(&props, "Model");
        let laptop = kind == 2 && number(&props, "PowerSupply") == 1;
        let name = if model.is_empty() {
            if laptop {
                "Laptop battery".into()
            } else {
                path.to_string()
                    .rsplit('/')
                    .next()
                    .unwrap_or("Device")
                    .to_owned()
            }
        } else if laptop {
            format!("Laptop · {model}")
        } else {
            model
        };
        let percent = props
            .get("Percentage")
            .and_then(|v| v.0.as_f64())
            .filter(|p| p.is_finite() && (0.0..=100.0).contains(p));
        let state = match number(&props, "State") {
            1 => "charging",
            2 => "discharging",
            3 => "empty",
            4 => "full",
            5 => "pending charge",
            6 => "pending discharge",
            _ => "connected",
        }
        .to_owned();
        let drain_watts = if number(&props, "State") == 2 {
            positive(&props, "EnergyRate")
        } else {
            None
        };
        devices.push(Device {
            name,
            native: {
                let native = text(&props, "NativePath");
                if native.is_empty() {
                    path.to_string()
                } else {
                    native
                }
            },
            percent,
            state,
            laptop,
            drain_watts,
            seconds_left: None,
            percent_per_hour: None,
        });
    }
    // Some BlueZ Battery1 devices are not exported by UPower yet.
    type Objects = HashMap<dbus::Path<'static>, HashMap<String, PropMap>>;
    let bluez = conn.with_proxy("org.bluez", "/", TIMEOUT);
    let objects: Result<(Objects,), _> = bluez.method_call(
        "org.freedesktop.DBus.ObjectManager",
        "GetManagedObjects",
        (),
    );
    if let Ok((objects,)) = objects {
        for (path, interfaces) in objects {
            let (Some(battery), Some(device)) = (
                interfaces.get("org.bluez.Battery1"),
                interfaces.get("org.bluez.Device1"),
            ) else {
                continue;
            };
            if number(device, "Connected") == 0 {
                continue;
            }
            let address = text(device, "Address");
            let normalize = |s: &str| s.replace('_', ":").to_lowercase();
            if devices.iter().any(|d| {
                d.native == path.to_string()
                    || (!address.is_empty() && normalize(&d.native).contains(&normalize(&address)))
            }) {
                continue;
            }
            let alias = text(device, "Alias");
            devices.push(Device {
                name: if alias.is_empty() { address } else { alias },
                native: path.to_string(),
                percent: battery
                    .get("Percentage")
                    .and_then(|v| v.0.as_u64())
                    .filter(|p| *p <= 100)
                    .map(|p| p as f64),
                state: "Bluetooth".into(),
                laptop: false,
                drain_watts: None,
                seconds_left: None,
                percent_per_hour: None,
            });
        }
    }
    devices.sort_by(|a, b| b.laptop.cmp(&a.laptop).then(a.name.cmp(&b.name)));
    Ok(devices)
}

pub fn spec(result: &Result<Vec<Device>>) -> (TraySpec<Action>, String) {
    let mut menu = vec![
        MenuEntry::Label("tybat · Batteries".into()),
        MenuEntry::Separator,
    ];
    let mut label = String::new();
    let mut icon = IconState::Normal;
    match result {
        Err(error) => menu.push(MenuEntry::Label(format!(
            "Battery discovery failed: {error}"
        ))),
        Ok(devices) => {
            if devices.is_empty() {
                menu.push(MenuEntry::Label("No batteries detected".into()));
            }
            for device in devices {
                let level = device
                    .percent
                    .map_or("unknown".into(), |p| format!("{p:.0}%"));
                menu.push(MenuEntry::Label(format!(
                    "{}: {level} · {}",
                    device.name, device.state
                )));
                if device.state == "discharging" {
                    let mut details = Vec::new();
                    if let Some(rate) = device.percent_per_hour {
                        details.push(format!("{rate:.1}%/h drain"));
                    }
                    if let Some(watts) = device.drain_watts {
                        details.push(format!("{watts:.1} W drain"));
                    }
                    if let Some(seconds) = device.seconds_left {
                        let time = remaining_time(seconds);
                        details.push(format!("~{time} remaining"));
                    } else {
                        details.push("Measuring drain…".into());
                    }
                    if !details.is_empty() {
                        menu.push(MenuEntry::Label(details.join(" · ")));
                    }
                }
            }
            if let Some(device) = devices
                .iter()
                .filter(|d| d.laptop && d.percent.is_some())
                .min_by(|a, b| {
                    a.percent
                        .partial_cmp(&b.percent)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            {
                let percent = device.percent.unwrap_or_default();
                label = format!("{percent:.0}%");
                if let Some(seconds) = device.seconds_left {
                    label.push_str(&format!(" · ~{}", remaining_time(seconds)));
                }
                icon = if device.state == "charging" {
                    IconState::Normal
                } else if percent <= 5.0 {
                    IconState::Attention
                } else if percent <= 20.0 {
                    IconState::Busy
                } else {
                    IconState::Normal
                };
            }
        }
    }
    (
        TraySpec {
            icon,
            title: format!("tybat {label}"),
            menu,
            click_action: None,
        },
        label,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measures_percentage_loss_and_resets_after_gaps_or_increases() {
        let mut history = History::default();
        for seconds in (0..600).step_by(10) {
            assert_eq!(history.observe(Duration::from_secs(seconds), 80.0), None);
        }
        assert_eq!(history.observe(Duration::from_secs(600), 78.0), Some(12.0));
        assert_eq!(history.observe(Duration::from_secs(610), 79.0), None);
        assert_eq!(history.observe(Duration::from_secs(1200), 70.0), None);
        assert_eq!(history.observe(Duration::from_secs(1000), 69.0), None);
    }

    #[test]
    fn tray_and_status_estimate_without_watts_and_clear_on_charging() {
        let mut tracker = Tracker::default();
        let mut result = Ok(vec![Device {
            name: "Laptop battery".into(),
            native: "BAT0".into(),
            percent: Some(80.0),
            state: "discharging".into(),
            laptop: true,
            drain_watts: None,
            seconds_left: None,
            percent_per_hour: None,
        }]);
        for seconds in (0..600).step_by(10) {
            tracker.update(&mut result, Duration::from_secs(seconds));
        }
        result.as_mut().unwrap()[0].percent = Some(78.0);
        tracker.update(&mut result, Duration::from_secs(600));
        assert_eq!(spec(&result).1, "78% · ~6h 30m");
        assert!(status(&result).contains("12.0%/h drain · ~6h 30m remaining"));
        result.as_mut().unwrap()[0].state = "charging".into();
        tracker.update(&mut result, Duration::from_secs(610));
        assert_eq!(spec(&result).1, "78%");
        result.as_mut().unwrap()[0].state = "discharging".into();
        tracker.update(&mut result, Duration::from_secs(620));
        assert!(status(&result).contains("Measuring drain"));
    }
}

pub fn status(result: &Result<Vec<Device>>) -> String {
    spec(result)
        .0
        .menu
        .into_iter()
        .filter_map(|entry| {
            if let MenuEntry::Label(line) = entry {
                Some(line.replace(['\n', '\r', '\t'], " "))
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\t")
}

pub fn start(
    charge_config: Option<crate::charge::Config>,
    status_cache: Arc<Mutex<Option<String>>>,
) -> Result<()> {
    use ty_common::tray::{self, Applet};
    let (send, receive) = tray::channel::<Action>();
    let icons = ty_common::icons::Tool::Bat.tray_icons();
    let (spec, _) = spec(&Ok(Vec::new()));
    let mut applet = Applet::new("tybat", &icons, spec, send)?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let (action_tx, action_rx) = std::sync::mpsc::channel();
    if charge_config.is_some() {
        crate::charge::watch_resume(action_tx.clone());
    }
    std::thread::spawn(move || {
        let mut charge = charge_config.map(crate::charge::Controller::new);
        let mut tracker = Tracker::default();
        loop {
            let mut result = scan();
            tracker.update(
                &mut result,
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default(),
            );
            *status_cache.lock().unwrap() = Some(status(&result));
            if tx
                .send((
                    result,
                    charge.as_ref().map(|c| c.menu()),
                    crate::profiles::menu(),
                ))
                .is_err()
            {
                break;
            }
            match action_rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Action::Charge(action)) => {
                    if let Some(charge) = &mut charge {
                        charge.handle(action);
                    }
                }
                Ok(Action::Profile(profile)) => crate::profiles::set(&profile),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
        while let Ok(action) = receive.try_recv() {
            let _ = action_tx.send(action);
        }
        if let Ok((result, charge_menu, profile_menu)) = rx.try_recv() {
            let (mut spec, label) = self::spec(&result);
            spec.menu.push(MenuEntry::Separator);
            spec.menu.push(profile_menu);
            if let Some(menu) = charge_menu {
                spec.menu.push(MenuEntry::Separator);
                spec.menu.push(menu);
            }
            applet.update(spec);
            applet.set_label(&label);
        }
        gtk::glib::ControlFlow::Continue
    });
    Ok(())
}
