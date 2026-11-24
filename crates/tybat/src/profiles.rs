use std::time::Duration;

use anyhow::Result;
use dbus::arg::PropMap;
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
use dbus::blocking::Connection;
use ty_common::tray::MenuEntry;

use crate::devices::Action;

const SERVICE: &str = "net.hadess.PowerProfiles";
const PATH: &str = "/net/hadess/PowerProfiles";
const TIMEOUT: Duration = Duration::from_secs(2);

fn label(profile: &str) -> &str {
    match profile {
        "power-saver" => "Power saver",
        "balanced" => "Balanced",
        "performance" => "Performance",
        other => other,
    }
}

fn read_menu() -> Result<MenuEntry<Action>> {
    let conn = Connection::new_system()?;
    let proxy = conn.with_proxy(SERVICE, PATH, TIMEOUT);
    let active: String = proxy.get(SERVICE, "ActiveProfile")?;
    let profiles: Vec<PropMap> = proxy.get(SERVICE, "Profiles")?;
    let entries = profiles
        .iter()
        .filter_map(|props| props.get("Profile")?.0.as_str())
        .map(|profile| MenuEntry::Action {
            icon: None,
            label: format!(
                "{}{}",
                if profile == active { "✓ " } else { "" },
                label(profile)
            ),
            enabled: profile != active,
            action: Action::Profile(profile.to_owned()),
        })
        .collect();
    Ok(MenuEntry::Submenu {
        icon: None,
        label: format!("Power profile: {}", label(&active)),
        entries,
    })
}

pub fn menu() -> MenuEntry<Action> {
    read_menu().unwrap_or_else(|_| MenuEntry::Label("Power profiles unavailable".into()))
}

pub fn set(profile: &str) {
    let result = (|| -> Result<()> {
        let conn = Connection::new_system()?;
        let proxy = conn.with_proxy(SERVICE, PATH, TIMEOUT);
        proxy.set(SERVICE, "ActiveProfile", profile.to_owned())?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("tybat: setting power profile: {error:#}");
        ty_common::notify::notify(
            "tybat",
            "Power profile could not be updated",
            &format!("{error:#}"),
            Duration::from_secs(10),
        );
    }
}
