//! Resumable, checksum-verified model downloads on a background thread.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::speech::models::ModelInfo;

const CHUNK: usize = 256 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Partial files being written. A cancelled download may still be finishing its last read
/// when the same model is requested again; the new one waits for it instead of racing it.
static BUSY_FILES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

struct FileLock(PathBuf);

impl FileLock {
    fn acquire(path: &Path, cancel: &AtomicBool) -> Result<Self> {
        loop {
            {
                let mut busy = BUSY_FILES.lock().unwrap();
                if !busy.iter().any(|other| other == path) {
                    busy.push(path.to_path_buf());
                    return Ok(Self(path.to_path_buf()));
                }
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(Cancelled.into());
            }
            thread::sleep(LOCK_POLL_INTERVAL);
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        BUSY_FILES.lock().unwrap().retain(|other| other != &self.0);
    }
}

#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// 0.0..=1.0.
    pub fn fraction(&self) -> f32 {
        match self.total() {
            0 => 0.0,
            total => (self.done() as f64 / total as f64).clamp(0.0, 1.0) as f32,
        }
    }
}

/// Returned (inside the error chain) when a download was cancelled.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("download cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub fn is_cancelled(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Cancelled>().is_some()
}

/// A download running on its own thread. Dropping it cancels the download; the partial file
/// is kept so the next attempt resumes.
pub struct Download {
    pub model: &'static ModelInfo,
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<PathBuf>>>,
}

impl Download {
    pub fn start(model: &'static ModelInfo, dir: PathBuf) -> Self {
        let progress = Arc::new(Progress::default());
        progress.total.store(model.bytes, Ordering::Relaxed);
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

    /// Wait for the result. Call once [`Self::is_finished`] is true to avoid blocking.
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

/// Download `model` into `dir` (blocking), reporting into `progress`. Returns the final path.
pub fn fetch(
    model: &ModelInfo,
    dir: &Path,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let path = model.path_in(dir);
    progress.total.store(model.bytes, Ordering::Relaxed);
    if model.is_downloaded(dir) {
        progress.done.store(model.bytes, Ordering::Relaxed);
        return Ok(path);
    }
    fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let partial = dir.join(format!("{}.part", model.filename));
    let _lock = FileLock::acquire(&partial, cancel)?;
    if model.is_downloaded(dir) {
        progress.done.store(model.bytes, Ordering::Relaxed);
        return Ok(path);
    }

    // Resume: hash what is already on disk, then ask for the rest.
    let mut hasher = Sha256::new();
    let mut have = hash_existing(&partial, &mut hasher, progress, cancel)?;
    if have > model.bytes {
        fs::remove_file(&partial)?;
        hasher = Sha256::new();
        have = 0;
    }

    if have < model.bytes {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(CONNECT_TIMEOUT))
            .http_status_as_error(false)
            .user_agent(concat!("tyspeak/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        let mut request = agent.get(model.url());
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let mut response = request
            .call()
            .with_context(|| format!("failed to reach {}", model.url()))?;
        let status = response.status().as_u16();
        let mut file = match status {
            206 if have > 0 => OpenOptions::new().append(true).open(&partial)?,
            200 => {
                // The server ignored the range; start over.
                hasher = Sha256::new();
                have = 0;
                File::create(&partial)?
            }
            _ => bail!("download of {} failed: HTTP {status}", model.filename),
        };
        progress.done.store(have, Ordering::Relaxed);

        let mut reader = response.body_mut().as_reader();
        let mut buffer = vec![0u8; CHUNK];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Cancelled.into());
            }
            let read = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err).context("download interrupted"),
            };
            file.write_all(&buffer[..read])
                .with_context(|| format!("failed to write {}", partial.display()))?;
            hasher.update(&buffer[..read]);
            have += read as u64;
            progress.done.store(have, Ordering::Relaxed);
        }
        file.sync_all()?;
    }

    if have != model.bytes {
        bail!(
            "download of {} ended early ({have} of {} bytes); try again to resume",
            model.filename,
            model.bytes
        );
    }
    let digest = hex(&hasher.finalize());
    if digest != model.sha256 {
        let _ = fs::remove_file(&partial);
        bail!(
            "checksum mismatch for {} (got {digest}, expected {})",
            model.filename,
            model.sha256
        );
    }
    fs::rename(&partial, &path)
        .with_context(|| format!("failed to move download to {}", path.display()))?;
    Ok(path)
}

fn hash_existing(
    partial: &Path,
    hasher: &mut Sha256,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<u64> {
    let Ok(mut file) = File::open(partial) else {
        return Ok(0);
    };
    let mut buffer = vec![0u8; CHUNK];
    let mut have = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(have);
        }
        hasher.update(&buffer[..read]);
        have += read as u64;
        progress.done.store(have, Ordering::Relaxed);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::{hex, Progress};
    use sha2::{Digest, Sha256};
    use std::sync::atomic::Ordering;

    #[test]
    fn hex_digest_matches_known_value() {
        assert_eq!(
            hex(&Sha256::digest(b"hello")),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn progress_fraction() {
        let progress = Progress::default();
        assert_eq!(progress.fraction(), 0.0);
        progress.total.store(200, Ordering::Relaxed);
        progress.done.store(50, Ordering::Relaxed);
        assert_eq!(progress.fraction(), 0.25);
    }
}
