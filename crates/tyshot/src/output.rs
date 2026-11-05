//! Saving screenshots to disk and desktop notifications.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use gtk::glib;

use crate::config::{NotificationsConfig, SaveConfig};

/// Write `png` to `<output_dir>/<base_name>-<timestamp>.png`, adding `-1`, `-2`, … if that
/// name is taken. Returns the path written.
pub fn save_png(png: &[u8], config: &SaveConfig) -> Result<PathBuf> {
    let dir = config.resolved_output_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let timestamp = glib::DateTime::now_local()
        .and_then(|now| now.format("%Y-%m-%d_%H-%M-%S"))
        .context("failed to format the current time")?;

    for attempt in 0.. {
        let name = match attempt {
            0 => format!("{}-{timestamp}.png", config.base_name),
            n => format!("{}-{timestamp}-{n}.png", config.base_name),
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(png)
                    .with_context(|| format!("failed to write {}", path.display()))?;
                return Ok(path);
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err).with_context(|| format!("failed to create {}", path.display()))
            }
        }
    }
    unreachable!()
}

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_ICON: &str = "image-x-generic";

/// Show a desktop notification (unless disabled in the config). Failures are only logged.
pub fn notify(config: &NotificationsConfig, summary: &str, body: &str, icon: Option<&Path>) {
    if !config.enabled {
        return;
    }
    let icon = icon.map_or_else(
        || DEFAULT_ICON.to_string(),
        |icon| icon.display().to_string(),
    );
    ty_common::notify::notify_with_icon("tyshot", summary, body, NOTIFY_TIMEOUT, Some(&icon));
}
