//! Location from GeoClue2 (system D-Bus), with the last fix cached for offline starts.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
use dbus::blocking::{Connection, Proxy};

use crate::config::APP_NAME;
use crate::solar::Location;

const SERVICE: &str = "org.freedesktop.GeoClue2";
const CLIENT: &str = "org.freedesktop.GeoClue2.Client";
const LOCATION: &str = "org.freedesktop.GeoClue2.Location";
const TIMEOUT: Duration = Duration::from_secs(5);
/// `GCLUE_ACCURACY_LEVEL_CITY`: plenty for sunrise and sunset.
const ACCURACY_CITY: u32 = 4;

/// A started GeoClue client. The client lives as long as the connection.
pub struct Client {
    conn: Connection,
    path: dbus::Path<'static>,
}

impl Client {
    pub fn start() -> Result<Self> {
        let conn = Connection::new_system().context("failed to connect to the system bus")?;
        let manager = conn.with_proxy(SERVICE, "/org/freedesktop/GeoClue2/Manager", TIMEOUT);
        let (path,): (dbus::Path<'static>,) = manager
            .method_call("org.freedesktop.GeoClue2.Manager", "GetClient", ())
            .context("GeoClue2 is not available (is the geoclue service enabled?)")?;
        let client = Self { conn, path };
        let proxy = client.proxy(client.path.clone());
        proxy.set(CLIENT, "DesktopId", APP_NAME.to_string())?;
        proxy.set(CLIENT, "RequestedAccuracyLevel", ACCURACY_CITY)?;
        proxy
            .method_call::<(), _, _, _>(CLIENT, "Start", ())
            .context(
                "GeoClue2 refused tylight; allow it in the geoclue config \
                 (see the tylight README) or set `location` in the tylight config",
            )?;
        Ok(client)
    }

    fn proxy(&self, path: dbus::Path<'static>) -> Proxy<'_, &Connection> {
        self.conn.with_proxy(SERVICE, path, TIMEOUT)
    }

    /// The current fix, if GeoClue has one yet.
    pub fn location(&self) -> Result<Option<Location>> {
        let path: dbus::Path<'static> = self.proxy(self.path.clone()).get(CLIENT, "Location")?;
        if &*path == "/" {
            return Ok(None);
        }
        let location = self.proxy(path);
        Ok(Some(Location {
            latitude: location.get(LOCATION, "Latitude")?,
            longitude: location.get(LOCATION, "Longitude")?,
        }))
    }

    /// Wait up to `timeout` for a fix.
    pub fn wait(&self, timeout: Duration) -> Result<Option<Location>> {
        let started = Instant::now();
        loop {
            if let Some(location) = self.location()? {
                return Ok(Some(location));
            }
            if started.elapsed() >= timeout {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(500));
        }
    }
}

/// Report GeoClue fixes to `on_fix` from a background thread, retrying if GeoClue is down.
pub fn watch(on_fix: impl Fn(Location) + Send + 'static) {
    thread::spawn(move || {
        let mut last_error = String::new();
        loop {
            match follow(&on_fix) {
                Ok(()) => {}
                Err(err) => {
                    let message = format!("{err:#}");
                    if message != last_error {
                        eprintln!("geoclue: {message}");
                        last_error = message;
                    }
                }
            }
            thread::sleep(Duration::from_secs(60));
        }
    });
}

fn follow(on_fix: &impl Fn(Location)) -> Result<()> {
    let client = Client::start()?;
    let mut last: Option<Location> = None;
    loop {
        if let Some(location) = client.location()? {
            let moved = last.is_none_or(|l| {
                (l.latitude - location.latitude).abs() > 0.01
                    || (l.longitude - location.longitude).abs() > 0.01
            });
            if moved {
                save_cache(location);
                on_fix(location);
                last = Some(location);
            }
        }
        let wait = if last.is_some() { 300 } else { 2 };
        thread::sleep(Duration::from_secs(wait));
    }
}

fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join(APP_NAME).join("location.json"))
}

pub fn load_cache() -> Option<Location> {
    let raw = fs::read_to_string(cache_path()?).ok()?;
    let location: Location = serde_json::from_str(&raw).ok()?;
    location.validate().ok()?;
    Some(location)
}

fn save_cache(location: Location) {
    let Some(path) = cache_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(&location) {
        let _ = fs::write(path, json);
    }
}
