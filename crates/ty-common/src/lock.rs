//! Single-instance lock: a Unix socket in `$XDG_RUNTIME_DIR`. A live listener means another
//! instance is running; a dead socket file is stale and removed. Apps that take commands
//! over a socket can reuse the bound listener as their control socket.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// How long either side of a [`serve`] / [`request`] exchange waits for the other.
const TIMEOUT: Duration = Duration::from_secs(2);

/// Answer one-line requests on `listener` from a background thread: each connection sends a
/// line and gets `handle(line)` back as one line.
pub fn serve(
    listener: &UnixListener,
    handle: impl Fn(&str) -> String + Send + 'static,
) -> Result<()> {
    let listener = listener.try_clone()?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(TIMEOUT));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let _ = writeln!(&stream, "{}", handle(line.trim()));
        }
    });
    Ok(())
}

/// Send `line` to the running `app` (see [`serve`]) and return its reply.
pub fn request(app: &str, line: &str) -> Result<String> {
    let stream =
        UnixStream::connect(socket_path(app)).with_context(|| format!("{app} isn't running"))?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    writeln!(&stream, "{line}")?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply)?;
    Ok(reply.trim().to_string())
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
    fn serves_one_line_requests() {
        let app = format!("ty-common-serve-test-{}", std::process::id());
        let lock = InstanceLock::acquire(&app).expect("lock");
        super::serve(lock.listener(), |line| format!("got {line}")).expect("serve");
        assert_eq!(super::request(&app, "status").unwrap(), "got status");
    }

    #[test]
    fn recovers_from_stale_socket_file() {
        let path = test_path("stale");
        drop(std::os::unix::net::UnixListener::bind(&path).expect("stale listener"));
        let lock = InstanceLock::acquire_at("test", path.clone()).expect("recover stale");
        drop(lock);
    }
}
