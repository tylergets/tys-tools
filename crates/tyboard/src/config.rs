use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::clipboard::secrets::SecretFilter;

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
    #[serde(default)]
    pub secrets: SecretsConfig,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            backend: ClipboardBackend::Auto,
            active_window: ActiveWindowConfig::default(),
            secrets: SecretsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardBackend {
    /// `gtk` on X11, `wl_clipboard` on Wayland.
    #[default]
    Auto,
    /// GTK's clipboard (X11).
    Gtk,
    /// `wl-paste --watch` (Wayland compositors with the data-control protocol).
    WlClipboard,
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

/// Text that looks like a secret is never added to history.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SecretsConfig {
    pub enabled: bool,
    /// Use the built-in patterns (API keys, tokens, private keys, URL credentials).
    pub builtin_patterns: bool,
    /// Extra regexes; text matching any of them is treated as a secret.
    pub patterns: Vec<String>,
}

impl Default for SecretsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            builtin_patterns: true,
            patterns: Vec::new(),
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
        SecretFilter::from_config(&self.secrets)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn parses_clipboard_backends() {
        let cfg: AppConfig =
            serde_json::from_str(r#"{"clipboard": {"backend": "wl_clipboard"}}"#).unwrap();
        assert_eq!(cfg.clipboard.backend, super::ClipboardBackend::WlClipboard);
        assert_eq!(
            AppConfig::default().clipboard.backend,
            super::ClipboardBackend::Auto
        );
    }

    #[test]
    fn validate_accepts_empty_config() {
        let cfg = AppConfig::default();
        cfg.validate().expect("empty config should be valid");
    }
}
