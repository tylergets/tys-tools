use std::path::PathBuf;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use ty_common::config::Loaded;

pub const APP_NAME: &str = "typk";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Failed attempts (wrong password, unrecognized finger) before the request is refused.
    pub max_attempts: u32,
    /// Show the action id and details expanded rather than behind "Details".
    pub show_details: bool,
    /// Darken the whole screen behind the prompt while it is open.
    pub dim_screen: bool,
    /// Path of polkit's `polkit-agent-helper-1`, for systems where it is neither reachable
    /// over polkit's helper socket nor at one of the usual places.
    pub helper: Option<PathBuf>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            show_details: false,
            dim_screen: false,
            helper: None,
        }
    }
}

impl AppConfig {
    pub fn load(path: Option<PathBuf>) -> Result<Loaded<Self>> {
        ty_common::config::load(APP_NAME, path)
    }

    pub fn validate(&self) -> Result<()> {
        if !(1..=20).contains(&self.max_attempts) {
            bail!("max_attempts must be between 1 and 20");
        }
        if let Some(helper) = &self.helper {
            if !helper.is_file() {
                bail!("helper {} does not exist", helper.display());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config() {
        let config: AppConfig =
            ty_common::config::parse("max_attempts: 5\nshow_details: true").unwrap();
        assert_eq!(config.max_attempts, 5);
        assert!(config.show_details);
        config.validate().unwrap();
    }

    #[test]
    fn blank_config_is_default() {
        let config: AppConfig = ty_common::config::parse("").unwrap();
        assert_eq!(config, AppConfig::default());
        config.validate().unwrap();
    }

    #[test]
    fn rejects_zero_attempts() {
        let config: AppConfig = ty_common::config::parse("max_attempts: 0").unwrap();
        assert!(config.validate().is_err());
    }
}
