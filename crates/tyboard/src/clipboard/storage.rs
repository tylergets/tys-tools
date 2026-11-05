use std::collections::VecDeque;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::clipboard::state::ClipboardEntry;

const CACHE_DIR_NAME: &str = "tyboard";
const HISTORY_FILE_NAME: &str = "history.json";

#[derive(Debug, Serialize, Deserialize)]
struct HistoryDatabase {
    history: Vec<ClipboardEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HistoryDatabaseCompat {
    Current(HistoryDatabase),
    Legacy { history: Vec<String> },
}

pub fn default_database_path() -> Result<PathBuf> {
    let cache_root = dirs::cache_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".cache")))
        .context("unable to resolve cache directory from environment")?;

    Ok(cache_root.join(CACHE_DIR_NAME).join(HISTORY_FILE_NAME))
}

pub fn load_history(path: &PathBuf, history_limit: usize) -> Result<VecDeque<ClipboardEntry>> {
    if !path.exists() {
        return Ok(VecDeque::new());
    }

    let raw = std::fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read clipboard history database: {}",
            path.display()
        )
    })?;
    let db: HistoryDatabaseCompat = serde_json::from_str(&raw).with_context(|| {
        format!(
            "failed to parse clipboard history database: {}",
            path.display()
        )
    })?;

    let mut history = VecDeque::new();
    match db {
        HistoryDatabaseCompat::Current(current) => {
            for item in current.history {
                if !item.is_empty() {
                    history.push_back(item);
                }
            }
        }
        HistoryDatabaseCompat::Legacy {
            history: old_entries,
        } => {
            for item in old_entries {
                if !item.is_empty() {
                    history.push_back(ClipboardEntry::Text {
                        value: item,
                        source_window: None,
                    });
                }
            }
        }
    }

    while history.len() > history_limit {
        history.pop_back();
    }

    Ok(history)
}

pub fn save_history(path: &PathBuf, history: &VecDeque<ClipboardEntry>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create clipboard history cache directory: {}",
                parent.display()
            )
        })?;
    }

    let db = HistoryDatabase {
        history: history.iter().cloned().collect(),
    };
    let raw = serde_json::to_string_pretty(&db).context("failed to serialize clipboard history")?;
    std::fs::write(path, raw).with_context(|| {
        format!(
            "failed to write clipboard history database: {}",
            path.display()
        )
    })?;

    Ok(())
}
