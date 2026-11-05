use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub clipboard: ClipboardConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClipboardConfig {
    #[serde(default)]
    pub backend: ClipboardBackend,
    #[serde(default)]
    pub active_window: ActiveWindowConfig,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            backend: ClipboardBackend::Gtk,
            active_window: ActiveWindowConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardBackend {
    #[default]
    Gtk,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActiveWindowConfig {
    #[serde(default)]
    pub backend: ActiveWindowBackend,
    #[serde(default)]
    pub blacklist: Vec<String>,
}

impl Default for ActiveWindowConfig {
    fn default() -> Self {
        Self {
            backend: ActiveWindowBackend::Auto,
            blacklist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActiveWindowBackend {
    #[default]
    Auto,
    Disabled,
    Command {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub config: AppConfig,
}

impl AppConfig {
    /// Load `--config`, or `./tyboard.yaml`, or `~/.config/tyboard/config.yaml`. Errors if
    /// none exists.
    pub fn load(config_path_override: Option<PathBuf>) -> Result<LoadedConfig> {
        let loaded =
            ty_common::config::load_required::<AppConfig>("tyboard", config_path_override)?;
        Ok(LoadedConfig {
            path: loaded.path.context("config loader returned no path")?,
            config: loaded.config,
        })
    }

    pub fn validate(&self) -> Result<()> {
        self.clipboard.validate()?;
        Ok(())
    }
}

impl ClipboardConfig {
    fn validate(&self) -> Result<()> {
        match &self.active_window.backend {
            ActiveWindowBackend::Command { program, .. } if program.trim().is_empty() => {
                bail!("clipboard.active_window.command program cannot be empty");
            }
            _ => {}
        }
        for (index, value) in self.active_window.blacklist.iter().enumerate() {
            if value.trim().is_empty() {
                bail!("clipboard.active_window.blacklist[{index}] cannot be empty");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn validate_accepts_empty_config() {
        let cfg = AppConfig::default();
        cfg.validate().expect("empty config should be valid");
    }
}
