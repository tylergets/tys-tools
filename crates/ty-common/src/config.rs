//! Config file discovery and loading, shared by every tool.
//!
//! Discovery order: `./<app>.yaml`, then `$XDG_CONFIG_HOME/<app>/config.yaml`. An explicit
//! path (`--config`, used by the Home Manager modules) skips discovery.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;

/// A parsed config and the file it came from (`None` when defaults are used).
#[derive(Debug, Clone)]
pub struct Loaded<T> {
    pub path: Option<PathBuf>,
    pub config: T,
}

/// The paths searched for `app`'s config, in order.
pub fn candidates(app: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(format!("{app}.yaml")));
    }
    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join(app).join("config.yaml"));
    }
    paths
}

/// The first existing config path for `app`.
pub fn discover(app: &str) -> Option<PathBuf> {
    candidates(app).into_iter().find(|path| path.exists())
}

/// Load `override_path`, or the discovered config, or `T::default()` if there is none.
pub fn load<T: DeserializeOwned + Default>(
    app: &str,
    override_path: Option<PathBuf>,
) -> Result<Loaded<T>> {
    match override_path.or_else(|| discover(app)) {
        Some(path) => Ok(Loaded {
            config: read(&path)?,
            path: Some(path),
        }),
        None => Ok(Loaded {
            path: None,
            config: T::default(),
        }),
    }
}

/// Like [`load`], but fails when no config file exists.
pub fn load_required<T: DeserializeOwned>(
    app: &str,
    override_path: Option<PathBuf>,
) -> Result<Loaded<T>> {
    let Some(path) = override_path.or_else(|| discover(app)) else {
        let list: Vec<String> = candidates(app)
            .iter()
            .map(|path| format!("- {}", path.display()))
            .collect();
        bail!(
            "no config file found; expected one of:\n{}",
            list.join("\n")
        );
    };
    Ok(Loaded {
        config: read(&path)?,
        path: Some(path),
    })
}

/// Parse a YAML file. An empty file parses as an empty mapping.
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    parse(&raw).with_context(|| format!("failed to parse YAML config: {}", path.display()))
}

/// Parse YAML text. Blank input parses as an empty mapping so all-default configs work.
pub fn parse<T: DeserializeOwned>(raw: &str) -> Result<T> {
    if raw.trim().is_empty() {
        return Ok(serde_yaml::from_str("{}")?);
    }
    Ok(serde_yaml::from_str(raw)?)
}

/// Replace a leading `~` with `home`.
pub fn expand_home(path: &Path, home: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => path.to_path_buf(),
    }
}

/// Replace a leading `~` with the user's home directory (unchanged if it is unknown).
pub fn expand_user(path: &Path) -> PathBuf {
    match dirs::home_dir() {
        Some(home) => expand_home(path, &home),
        None => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, serde::Deserialize, PartialEq)]
    #[serde(default)]
    struct Sample {
        name: String,
    }

    #[test]
    fn blank_yaml_is_default() {
        assert_eq!(parse::<Sample>("  \n").unwrap(), Sample::default());
    }

    #[test]
    fn expands_tilde() {
        let home = Path::new("/home/me");
        assert_eq!(expand_home(Path::new("~/x"), home), Path::new("/home/me/x"));
        assert_eq!(expand_home(Path::new("/abs"), home), Path::new("/abs"));
    }
}
