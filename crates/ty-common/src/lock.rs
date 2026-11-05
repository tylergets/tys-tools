//! Single-instance lock: a Unix socket in `$XDG_RUNTIME_DIR`. A live listener means another
//! instance is running; a dead socket file is stale and removed. Apps that take commands
//! over a socket can reuse the bound listener as their control socket.

use std::fs;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

pub struct InstanceLock {
    path: PathBuf,
    listener: UnixListener,
}

impl InstanceLock {
    /// Take the lock for `app` at [`socket_path`].
    pub fn acquire(app: &str) -> Result<Self> {
        Self::acquire_at(app, socket_path(app))
    }

    pub fn acquire_at(app: &str, path: PathBuf) -> Result<Self> {
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                bail!(
                    "another {app} instance is already running (lock: {})",
                    path.display()
                );
            }
            fs::remove_file(&path).with_context(|| {
                format!(
                    "failed to remove stale {app} instance lock: {}",
                    path.display()
                )
            })?;
        }

        let listener = UnixListener::bind(&path).with_context(|| {
            format!(
                "failed to create {app} instance lock socket: {}",
                path.display()
            )
        })?;
        Ok(Self { path, listener })
    }

    /// The bound socket, for apps that serve requests on it.
    pub fn listener(&self) -> &UnixListener {
        &self.listener
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// `$XDG_RUNTIME_DIR/<app>-<user>.sock`.
pub fn socket_path(app: &str) -> PathBuf {
    crate::runtime_dir().join(format!("{app}-{}.sock", crate::user_hint()))
}

#[cfg(test)]
mod tests {
    use super::InstanceLock;
    use std::path::PathBuf;

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ty-common-lock-{}-{name}.sock", std::process::id()))
    }

    #[test]
    fn rejects_second_lock_holder() {
        let path = test_path("second");
        let first = InstanceLock::acquire_at("test", path.clone()).expect("first lock");
        assert!(InstanceLock::acquire_at("test", path.clone()).is_err());
        drop(first);
        assert!(!path.exists(), "lock file removed on drop");
    }

    #[test]
    fn recovers_from_stale_socket_file() {
        let path = test_path("stale");
        drop(std::os::unix::net::UnixListener::bind(&path).expect("stale listener"));
        let lock = InstanceLock::acquire_at("test", path.clone()).expect("recover stale");
        drop(lock);
    }
}
