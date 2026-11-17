//! Model-specific background jobs over the shared resumable downloader.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use anyhow::{bail, Result};
pub use ty_common::download::{is_cancelled, Progress};

use crate::speech::models::ModelInfo;

/// Dropping a job cancels it; the shared downloader retains its partial file.
pub struct Download {
    pub model: &'static ModelInfo,
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<PathBuf>>>,
}

impl Download {
    pub fn start(model: &'static ModelInfo, dir: PathBuf) -> Self {
        let progress = Arc::new(Progress::new(model.bytes));
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = {
            let (progress, cancel) = (progress.clone(), cancel.clone());
            thread::spawn(move || fetch(model, &dir, &progress, &cancel))
        };
        Self {
            model,
            progress,
            cancel,
            handle: Some(handle),
        }
    }

    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Call after `is_finished` to avoid blocking the UI.
    pub fn join(mut self) -> Result<PathBuf> {
        match self.handle.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            Some(Err(_)) => bail!("download thread panicked"),
            None => bail!("download already joined"),
        }
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn fetch(
    model: &ModelInfo,
    dir: &Path,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    ty_common::download::fetch(
        &ty_common::download::Asset {
            url: &model.url(),
            path: &model.path_in(dir),
            bytes: model.bytes,
            sha256: model.sha256,
        },
        progress,
        cancel,
        |_| {},
    )
}

/// Check a model rejected by the loader and repair it if its contents are corrupt.
pub fn repair(model: &ModelInfo, dir: &Path) -> Result<PathBuf> {
    ty_common::download::fetch_verified(
        &ty_common::download::Asset {
            url: &model.url(),
            path: &model.path_in(dir),
            bytes: model.bytes,
            sha256: model.sha256,
        },
        &Progress::new(model.bytes),
        &AtomicBool::new(false),
        |_| {},
    )
}
