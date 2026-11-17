//! Unix process signaling with explicit process and process-group targets.

use std::io;

/// Signal one process. Zero and IDs outside the positive `pid_t` range are rejected.
pub fn signal(pid: u32, signal: i32) -> io::Result<()> {
    send(positive_pid(pid)?, signal)
}

/// Signal every process in a group. The group ID must be positive.
pub fn signal_group(pgid: u32, signal: i32) -> io::Result<()> {
    send(-positive_pid(pgid)?, signal)
}

fn positive_pid(pid: u32) -> io::Result<libc::pid_t> {
    libc::pid_t::try_from(pid)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "pid must be positive and fit pid_t",
            )
        })
}

fn send(target: libc::pid_t, signal: i32) -> io::Result<()> {
    // SAFETY: kill has no memory-safety preconditions. Callers validate the target
    // before applying process-group semantics; the kernel validates the signal.
    if unsafe { libc::kill(target, signal) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
