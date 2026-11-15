//! Resumable, SHA-256-verified downloads shared by the speech tools.
//!
//! Incomplete transfers stay in `<filename>.part`. An OS lock on
//! `<filename>.download.lock` serializes writers across threads and processes;
//! the lock file remains on disk, but its lock is released on exit or a crash.
//! Only a complete, verified file is atomically renamed to the destination.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

pub struct Asset<'a> {
    pub url: &'a str,
    pub path: &'a Path,
    pub bytes: u64,
    pub sha256: &'a str,
}

#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    pub fn new(total: u64) -> Self {
        Self {
            done: AtomicU64::new(0),
            total: AtomicU64::new(total),
        }
    }
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }
    pub fn fraction(&self) -> f32 {
        match self.total() {
            0 => 0.0,
            total => (self.done() as f64 / total as f64).clamp(0.0, 1.0) as f32,
        }
    }
}

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

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Cancelled.into());
    }
    Ok(())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

/// Existing published files use a size check; SHA-256 is verified at publication.
pub fn is_complete(path: &Path, bytes: u64) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == bytes)
}

/// Blocking fetch. Cancellation is checked between reads and while waiting for
/// the file lock. `report` receives absolute byte counts, including resumed bytes.
pub fn fetch(
    asset: &Asset<'_>,
    progress: &Progress,
    cancel: &AtomicBool,
    report: impl Fn(u64),
) -> Result<PathBuf> {
    progress.total.store(asset.bytes, Ordering::Relaxed);
    let update = |done| {
        progress.done.store(done, Ordering::Relaxed);
        report(done);
    };
    check_cancel(cancel)?;
    if is_complete(asset.path, asset.bytes) {
        update(asset.bytes);
        return Ok(asset.path.to_owned());
    }
    let parent = asset.path.parent().context("download path has no parent")?;
    fs::create_dir_all(parent)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(sibling(asset.path, ".download.lock"))?;
    loop {
        check_cancel(cancel)?;
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(100)),
            Err(TryLockError::Error(err)) => return Err(err).context("locking download"),
        }
    }
    // Another downloader may have finished while we waited.
    if is_complete(asset.path, asset.bytes) {
        update(asset.bytes);
        return Ok(asset.path.to_owned());
    }
    let partial = sibling(asset.path, ".part");
    for attempt in 0..2 {
        check_cancel(cancel)?;
        let digest = fetch_partial(asset, &partial, cancel, &update).with_context(|| {
            format!("downloading {}; run again to resume", asset.path.display())
        })?;
        check_cancel(cancel)?;
        if digest == asset.sha256 {
            File::open(&partial)?.sync_all()?;
            fs::rename(&partial, asset.path)?;
            return Ok(asset.path.to_owned());
        }
        // A stale/corrupt prefix cannot be repaired with a Range request.
        fs::remove_file(&partial)?;
        update(0);
        if attempt == 1 {
            bail!(
                "checksum mismatch for {} after a fresh download (got {digest}, expected {})",
                asset.path.display(),
                asset.sha256
            );
        }
    }
    unreachable!()
}

fn fetch_partial(
    asset: &Asset<'_>,
    partial: &Path,
    cancel: &AtomicBool,
    update: &impl Fn(u64),
) -> Result<String> {
    if fs::metadata(partial).is_ok_and(|m| m.len() > asset.bytes) {
        fs::remove_file(partial)?;
    }
    let mut hasher = Sha256::new();
    let mut have = 0;
    let mut buffer = [0u8; 65536];
    match File::open(partial) {
        Ok(mut file) => loop {
            check_cancel(cancel)?;
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            have += count as u64;
            update(have);
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    update(have);
    if have < asset.bytes {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(1800)))
            .http_status_as_error(false)
            .build()
            .into();
        // Range offsets refer to the actual file, not a compressed representation.
        let mut request = agent.get(asset.url).header("Accept-Encoding", "identity");
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        check_cancel(cancel)?;
        let mut response = request.call()?;
        if response
            .headers()
            .get("content-encoding")
            .is_some_and(|v| v != "identity")
        {
            bail!("server returned encoded content for a byte-range download");
        }
        let status = response.status().as_u16();
        let end = match status {
            200 => asset.bytes,
            206 if have > 0 => {
                let range = response
                    .headers()
                    .get("content-range")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.strip_prefix("bytes "))
                    .and_then(|s| s.split_once('/'))
                    .and_then(|(range, total)| {
                        let (start, end) = range.split_once('-')?;
                        Some((
                            start.parse::<u64>().ok()?,
                            end.parse::<u64>().ok()?,
                            total.parse::<u64>().ok()?,
                        ))
                    })
                    .context("missing or invalid Content-Range")?;
                if range.0 != have
                    || range.1 < have
                    || range.1 >= asset.bytes
                    || range.2 != asset.bytes
                {
                    bail!("server returned an inconsistent Content-Range");
                }
                range.1 + 1
            }
            _ => bail!("HTTP {status}"),
        };
        if let Some(length) = response.headers().get("content-length") {
            let length: u64 = length.to_str()?.parse()?;
            let expected = if status == 200 {
                asset.bytes
            } else {
                end - have
            };
            if length != expected {
                bail!("server returned an unexpected Content-Length");
            }
        }
        check_cancel(cancel)?;
        let mut file = if status == 200 {
            // Servers without Range support return the whole object.
            hasher = Sha256::new();
            have = 0;
            File::create(partial)?
        } else {
            OpenOptions::new().append(true).open(partial)?
        };
        update(have);
        let mut reader = response.body_mut().as_reader();
        loop {
            check_cancel(cancel)?;
            let count = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e).context("transfer interrupted"),
            };
            if count as u64 > end - have {
                bail!("response exceeds its expected size");
            }
            file.write_all(&buffer[..count])?;
            hasher.update(&buffer[..count]);
            have += count as u64;
            update(have);
        }
        file.sync_all()?;
    }
    if have != asset.bytes {
        bail!("transfer ended early ({have} of {} bytes)", asset.bytes);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
