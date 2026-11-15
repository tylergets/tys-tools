use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use dbus::arg::PropMap;
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
use dbus::blocking::Connection;
use ty_common::tray::{IconState, MenuEntry, TraySpec};

const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct Device {
    name: String,
    native: String,
    percent: Option<f64>,
    state: String,
    laptop: bool,
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
        devices.push(Device {
            name,
            native: text(&props, "NativePath"),
            percent,
            state,
            laptop,
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
            });
        }
    }
    devices.sort_by(|a, b| b.laptop.cmp(&a.laptop).then(a.name.cmp(&b.name)));
    Ok(devices)
}

pub fn spec(result: &Result<Vec<Device>>) -> (TraySpec<crate::charge::Action>, String) {
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

pub fn start(charge_config: Option<crate::charge::Config>) -> Result<()> {
    use ty_common::tray::{self, Applet};
    let (send, receive) = tray::channel::<crate::charge::Action>();
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
        loop {
            if tx
                .send((scan(), charge.as_ref().map(|c| c.menu())))
                .is_err()
            {
                break;
            }
            match action_rx.recv_timeout(Duration::from_secs(10)) {
                Ok(action) => {
                    if let Some(charge) = &mut charge {
                        charge.handle(action);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
        while let Ok(action) = receive.try_recv() {
            let _ = action_tx.send(action);
        }
        if let Ok((result, charge_menu)) = rx.try_recv() {
            let (mut spec, label) = self::spec(&result);
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
