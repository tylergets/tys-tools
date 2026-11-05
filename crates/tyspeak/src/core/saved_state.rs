//! State kept across restarts, in `$XDG_STATE_HOME/tyspeak/state.json`: the model picked in
//! the tray (or over D-Bus), which takes precedence over the config's `model`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

fn path() -> Option<PathBuf> {
    dirs::state_dir().map(|dir| dir.join("tyspeak").join("state.json"))
}

pub fn load() -> SavedState {
    path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(state: &SavedState) -> Result<()> {
    let path = path().context("no state directory")?;
    let dir = path.parent().context("invalid state path")?;
    fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    // Write then rename, so a crash never leaves a truncated file.
    let partial = path.with_extension("json.part");
    fs::write(&partial, serde_json::to_vec_pretty(state)?)
        .with_context(|| format!("failed to write {}", partial.display()))?;
    fs::rename(&partial, &path).with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}
