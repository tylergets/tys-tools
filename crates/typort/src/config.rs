use std::path::PathBuf;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use ty_common::config::Loaded;

use crate::ports::Port;

pub const APP_NAME: &str = "typort";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Ports never shown.
    pub ignore_ports: Vec<u16>,
    /// Programs never shown, by name as `typort list` prints it (case-insensitive).
    pub ignore_processes: Vec<String>,
    /// Ports opened with `https://`.
    pub https_ports: Vec<u16>,
    /// Fetch each port's page `<title>` with an HTTP request. Off by default.
    pub fetch_titles: bool,
    /// Command that opens URLs (the URL is appended). Default: `xdg-open`.
    pub browser: Option<Vec<String>>,
    /// How often the tray rescans, in seconds.
    pub refresh_seconds: f64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ignore_ports: Vec::new(),
            ignore_processes: vec!["kdeconnectd".into()],
            https_ports: Vec::new(),
            fetch_titles: false,
            browser: None,
            refresh_seconds: 2.0,
        }
    }
}

impl AppConfig {
    pub fn load(path: Option<PathBuf>) -> Result<Loaded<Self>> {
        ty_common::config::load(APP_NAME, path)
    }

    pub fn validate(&self) -> Result<()> {
        if !(0.25..=3600.0).contains(&self.refresh_seconds) {
            bail!("refresh_seconds must be between 0.25 and 3600");
        }
        if self.browser.as_ref().is_some_and(|b| b.is_empty()) {
            bail!("browser must not be empty (leave it out to use xdg-open)");
        }
        Ok(())
    }

    /// Whether `port` is hidden by the ignore lists.
    pub fn ignored(&self, port: &Port) -> bool {
        self.ignore_ports.contains(&port.port)
            || port.processes.iter().any(|p| {
                self.ignore_processes
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&p.name))
            })
    }

    pub fn url(&self, port: &Port) -> String {
        port.url(self.https_ports.contains(&port.port))
    }

    pub fn browser_command(&self) -> Vec<String> {
        self.browser
            .clone()
            .unwrap_or_else(|| vec!["xdg-open".into()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config() {
        let config: AppConfig = ty_common::config::parse(
            "ignore_ports: [631]\nbrowser: [firefox, --new-tab]\nhttps_ports: [8443]",
        )
        .unwrap();
        assert_eq!(config.ignore_ports, vec![631]);
        assert_eq!(config.browser_command(), vec!["firefox", "--new-tab"]);
        config.validate().unwrap();
    }

    #[test]
    fn blank_config_is_default() {
        let config: AppConfig = ty_common::config::parse("").unwrap();
        assert_eq!(config, AppConfig::default());
    }
}
