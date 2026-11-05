//! Config file discovery and loading, shared by every tool.
//!
//! Discovery order: `./<app>.yaml`, then `$XDG_CONFIG_HOME/<app>/config.yaml`. An explicit
//! path (`--config`, used by the Home Manager modules) skips discovery.
//!
//! Every tool's config may have a top-level `env:` map of environment variables to set for
//! the tool and the programs it runs. A value is a string, or `{ file: <path> }` to read it
//! from a file (trimmed), e.g. a secret:
//!
//! ```yaml
//! env:
//!   GH_TOKEN: { file: /run/secrets/github }
//!   LANG: en_US.UTF-8
//! ```
//!
//! The variables are set when a config file is first loaded (before the tool starts threads);
//! later reloads don't change them. They override the inherited environment.
//!
//! Likewise a top-level `appearance:` section (see [`crate::appearance`]) is split off and
//! kept in `appearance::current()`; every load replaces it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::appearance::{self, Appearance};

/// The shared top-level keys, removed before a tool's own config is parsed.
const ENV_KEY: &str = "env";
const APPEARANCE_KEY: &str = "appearance";

/// Whether `env:` was applied already (only the first load sets variables).
static ENV_APPLIED: AtomicBool = AtomicBool::new(false);

/// One `env:` value.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum EnvValue {
    Value(String),
    File { file: PathBuf },
}

impl EnvValue {
    fn resolve(&self) -> Result<String> {
        match self {
            EnvValue::Value(value) => Ok(value.clone()),
            EnvValue::File { file } => {
                let path = expand_user(file);
                let value = std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read env file {}", path.display()))?;
                Ok(value.trim().to_string())
            }
        }
    }
}

pub type Env = BTreeMap<String, EnvValue>;

/// The sections every tool's config shares.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shared {
    pub env: Env,
    pub appearance: Appearance,
}

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

/// Parse a YAML file and apply its shared sections: `env:` on the first load only,
/// `appearance:` on every load. An empty file parses as an empty mapping.
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    let (config, shared) = parse_with_shared(&raw)
        .with_context(|| format!("failed to parse YAML config: {}", path.display()))?;
    if !ENV_APPLIED.swap(true, Ordering::SeqCst) {
        apply_env(&shared.env).with_context(|| format!("in config {}", path.display()))?;
    }
    appearance::set(shared.appearance);
    Ok(config)
}

/// Parse YAML text. Blank input parses as an empty mapping so all-default configs work.
/// The shared sections are accepted and validated but not applied.
pub fn parse<T: DeserializeOwned>(raw: &str) -> Result<T> {
    Ok(parse_with_shared(raw)?.0)
}

/// Parse YAML text into the tool's config and the shared sections.
pub fn parse_with_shared<T: DeserializeOwned>(raw: &str) -> Result<(T, Shared)> {
    let mut value: serde_yaml_ng::Value = if raw.trim().is_empty() {
        serde_yaml_ng::Value::Mapping(Default::default())
    } else {
        serde_yaml_ng::from_str(raw)?
    };
    let mut take = |key| value.as_mapping_mut().and_then(|map| map.remove(key));
    let env = match take(ENV_KEY) {
        Some(env) => serde_yaml_ng::from_value(env)
            .context("`env` must map variable names to strings or { file: <path> }")?,
        None => Env::new(),
    };
    let appearance: Appearance = match take(APPEARANCE_KEY) {
        Some(appearance) => serde_yaml_ng::from_value(appearance).context("in `appearance`")?,
        None => Appearance::default(),
    };
    appearance.validate()?;
    let shared = Shared { env, appearance };
    Ok((serde_yaml_ng::from_value(value)?, shared))
}

/// Set every variable in `env`.
pub fn apply_env(env: &Env) -> Result<()> {
    // Resolve everything first so a bad entry doesn't leave the environment half set.
    let resolved = env
        .iter()
        .map(|(name, value)| {
            if name.is_empty() || name.contains(['=', '\0']) {
                bail!("invalid env variable name {name:?}");
            }
            Ok((name, value.resolve()?))
        })
        .collect::<Result<Vec<_>>>()?;
    for (name, value) in resolved {
        std::env::set_var(name, value);
    }
    Ok(())
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
    fn env_section_is_split_off() {
        let dir = std::env::temp_dir().join(format!("ty-common-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let secret = dir.join("secret");
        std::fs::write(&secret, "s3cret\n").unwrap();
        let raw = format!(
            "name: x\nenv:\n  PLAIN: hi\n  SECRET: {{ file: {} }}\n",
            secret.display()
        );
        let (config, shared): (Sample, Shared) = parse_with_shared(&raw).unwrap();
        assert_eq!(config.name, "x");
        assert_eq!(shared.env["PLAIN"], EnvValue::Value("hi".into()));
        assert_eq!(shared.env["SECRET"].resolve().unwrap(), "s3cret");
        assert!(parse::<Strict>("env: { A: b }").is_ok());
        assert!(parse::<Strict>("other: 1").is_err());
        assert!(parse::<Sample>("env: [nope]").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[derive(Debug, Default, serde::Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct Strict {}

    #[test]
    fn appearance_section_is_split_off() {
        let (config, shared): (Sample, Shared) =
            parse_with_shared("name: x\nappearance:\n  font: Iosevka\n").unwrap();
        assert_eq!(config.name, "x");
        assert_eq!(shared.appearance.font.as_deref(), Some("Iosevka"));
        assert!(parse::<Strict>("appearance: { font: Iosevka }").is_ok());
        assert!(parse::<Strict>("appearance: {}").is_ok());
        assert!(parse::<Strict>("appearance: { font: '' }").is_err());
        assert!(parse::<Strict>("appearance: { fnot: x }").is_err());
    }

    #[test]
    fn expands_tilde() {
        let home = Path::new("/home/me");
        assert_eq!(expand_home(Path::new("~/x"), home), Path::new("/home/me/x"));
        assert_eq!(expand_home(Path::new("/abs"), home), Path::new("/abs"));
    }
}
