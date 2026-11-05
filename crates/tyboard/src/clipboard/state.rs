use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::clipboard::storage;
use crate::core::active_window::ActiveWindowContext;

pub const DEFAULT_HISTORY_LIMIT: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClipboardEntry {
    Text {
        value: String,
        #[serde(default)]
        source_window: Option<ActiveWindowContext>,
    },
    Image {
        width: i32,
        height: i32,
        rowstride: i32,
        has_alpha: bool,
        bits_per_sample: i32,
        channels: i32,
        pixels: Vec<u8>,
        #[serde(default)]
        source_window: Option<ActiveWindowContext>,
    },
}

impl ClipboardEntry {
    pub fn is_empty(&self) -> bool {
        match self {
            ClipboardEntry::Text { value, .. } => value.is_empty(),
            ClipboardEntry::Image { pixels, .. } => pixels.is_empty(),
        }
    }

    pub fn with_source_window(mut self, source_window: Option<ActiveWindowContext>) -> Self {
        match &mut self {
            ClipboardEntry::Text {
                source_window: existing,
                ..
            } => *existing = source_window,
            ClipboardEntry::Image {
                source_window: existing,
                ..
            } => *existing = source_window,
        }
        self
    }
}

#[derive(Clone)]
pub struct SharedClipboardState {
    inner: Arc<Mutex<ClipboardState>>,
}

impl SharedClipboardState {
    pub fn load_default(history_limit: usize) -> Result<Self> {
        let state = ClipboardState::load_default(history_limit)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(state)),
        })
    }

    pub fn record_entry(&self, value: ClipboardEntry) -> Result<bool> {
        let mut guard = self.inner.lock().expect("clipboard state mutex poisoned");
        guard.record_entry(value)
    }

    pub fn history_snapshot(&self) -> Vec<ClipboardEntry> {
        let guard = self.inner.lock().expect("clipboard state mutex poisoned");
        guard.history_snapshot()
    }

    pub fn clear_history(&self) -> Result<()> {
        let mut guard = self.inner.lock().expect("clipboard state mutex poisoned");
        guard.clear_history()
    }
}

pub struct ClipboardState {
    database_path: PathBuf,
    history: VecDeque<ClipboardEntry>,
    history_limit: usize,
}

impl ClipboardState {
    pub fn load_default(history_limit: usize) -> Result<Self> {
        let database_path = storage::default_database_path()?;
        let history = storage::load_history(&database_path, history_limit)?;
        Ok(Self {
            database_path,
            history,
            history_limit,
        })
    }

    pub fn history_snapshot(&self) -> Vec<ClipboardEntry> {
        self.history.iter().cloned().collect()
    }

    pub fn record_entry(&mut self, value: ClipboardEntry) -> Result<bool> {
        if !push_history_entry(&mut self.history, self.history_limit, value) {
            return Ok(false);
        }

        storage::save_history(&self.database_path, &self.history)?;
        Ok(true)
    }

    pub fn clear_history(&mut self) -> Result<()> {
        self.history.clear();
        storage::save_history(&self.database_path, &self.history)
    }
}

fn push_history_entry(
    history: &mut VecDeque<ClipboardEntry>,
    history_limit: usize,
    value: ClipboardEntry,
) -> bool {
    if value.is_empty() {
        return false;
    }

    if let Some(index) = history.iter().position(|entry| entry == &value) {
        if index == 0 {
            return false;
        }
        history.remove(index);
    }

    history.push_front(value);
    while history.len() > history_limit {
        history.pop_back();
    }
    true
}
