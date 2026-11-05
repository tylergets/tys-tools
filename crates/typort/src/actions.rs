//! Finding, stopping and opening ports.

use std::process::{Command, Stdio};
use std::thread;

use anyhow::{bail, Context, Result};

use crate::config::AppConfig;
use crate::ports::{self, Port};

/// The visible ports (`all` includes ignored ones).
pub fn list(config: &AppConfig, all: bool) -> Result<Vec<Port>> {
    let mut ports = ports::scan()?;
    if !all {
        ports.retain(|port| !config.ignored(port));
    }
    Ok(ports)
}

/// The port, scanned fresh.
pub fn find(port: u16) -> Result<Port> {
    ports::scan()?
        .into_iter()
        .find(|p| p.port == port)
        .with_context(|| format!("nothing of yours is listening on port {port}"))
}

/// Send SIGTERM (or SIGKILL with `force`) to every process holding the port.
pub fn kill(port: &Port, force: bool) -> Result<()> {
    if port.processes.is_empty() {
        bail!("couldn't find the process listening on port {}", port.port);
    }
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    for pid in port.pids() {
        let pid = libc::pid_t::try_from(pid).context("pid out of range")?;
        // SAFETY: kill(2) has no memory-safety requirements.
        if unsafe { libc::kill(pid, signal) } != 0 {
            let err = std::io::Error::last_os_error();
            // Already gone (e.g. a worker that exited with its parent) is fine.
            if err.raw_os_error() != Some(libc::ESRCH) {
                return Err(err).with_context(|| format!("failed to signal pid {pid}"));
            }
        }
    }
    Ok(())
}

/// Open the port's URL in the browser, without waiting for it.
pub fn open(config: &AppConfig, port: &Port) -> Result<()> {
    let url = config.url(port);
    let command = config.browser_command();
    let mut child = Command::new(&command[0])
        .args(&command[1..])
        .arg(&url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to run {}", command[0]))?;
    // Reap it in the background so it doesn't linger as a zombie.
    thread::spawn(move || child.wait());
    Ok(())
}
