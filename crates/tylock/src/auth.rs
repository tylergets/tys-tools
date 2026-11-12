//! Authentication in the background, so drawing and typing never wait on PAM: a password
//! thread that checks each submitted password, and a fingerprint thread that keeps a
//! fingerprint-only PAM transaction running while the screen is locked. Whichever succeeds
//! first unlocks.

use std::ffi::CStr;
use std::mem::MaybeUninit;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use dbus::blocking::Connection;
use ty_common::screenlock::Waker;

use crate::pam::{self, Message, Secret};

/// Events from the auth threads to the lock screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The result of a submitted password; on failure, what to show.
    Password(Result<(), String>),
    Finger(Finger),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finger {
    /// The reader is there and you have prints: show the fingerprint.
    Available,
    /// A scan didn't match, or needs another try; PAM's message.
    Retry(String),
    Matched,
    /// No reader, no enrolled prints, or the PAM service doesn't work.
    Unavailable,
}

/// Start the password thread. Send it passwords; each gets an [`Event::Password`].
pub fn spawn_password(
    service: String,
    user: String,
    events: Sender<Event>,
    waker: Waker,
) -> Sender<Secret> {
    let (submit, passwords) = mpsc::channel::<Secret>();
    thread::spawn(move || {
        for password in passwords {
            let mut shown: Option<String> = None;
            let result = pam::authenticate(&service, &user, Some(&password), &mut |message| {
                // pam_faillock and friends explain themselves with error messages.
                if let Message::Error(text) = message {
                    if !text.is_empty() {
                        shown = Some(text);
                    }
                }
            });
            drop(password);
            let result = result.map_err(|err| {
                eprintln!("tylock: password not accepted: {err}");
                shown.unwrap_or_else(|| "Wrong password".to_string())
            });
            if events.send(Event::Password(result)).is_err() {
                return;
            }
            waker.wake();
        }
    });
    submit
}

/// Wait this long after locking before reading fingers, so the finger that just pressed a
/// power-button reader to lock doesn't unlock right away.
const START_DELAY: Duration = Duration::from_secs(1);
/// Attempts ending this fast mean the reader or the PAM service isn't working right now.
const QUICK_FAILURE: Duration = Duration::from_secs(2);
const RETRY_PAUSE: Duration = Duration::from_millis(500);
const MAX_RETRY_PAUSE: Duration = Duration::from_secs(10);
/// How often to look again when there is no reader or no prints (a reader can come back
/// after suspend or be plugged in).
const UNAVAILABLE_PAUSE: Duration = Duration::from_secs(20);

/// Whether `user` has prints enrolled, asking fprintd for at most `within` (fprintd may
/// need starting); used before locking, so the first frame already shows the right thing.
pub fn probe_enrolled(user: &str, within: Duration) -> bool {
    let (tx, rx) = mpsc::channel();
    let user = user.to_string();
    thread::spawn(move || {
        let _ = tx.send(enrolled(&user));
    });
    rx.recv_timeout(within).unwrap_or(false)
}

/// Whether a pam_fprintd message is about the scan itself (no match, scan again), which
/// is worth showing. Timeouts, busy readers and the like are retried out of sight.
fn is_scan_feedback(message: &str) -> bool {
    let message = message.to_lowercase();
    message.contains("match") || message.contains("again")
}

/// Start the fingerprint thread: run `service` (a PAM stack with only pam_fprintd) again
/// and again until a finger matches. `available` is what the screen shows already.
/// Whether the fingerprint is offered follows fprintd's enrolled prints only; PAM
/// failures (timeouts, a busy reader) are retried quietly so the screen doesn't flicker.
pub fn spawn_fingerprint(
    service: String,
    user: String,
    mut available: bool,
    events: Sender<Event>,
    waker: Waker,
) {
    thread::spawn(move || {
        let send = |event: Finger| {
            let ok = events.send(Event::Finger(event)).is_ok();
            waker.wake();
            ok
        };
        let mut first = true;
        let mut pause = RETRY_PAUSE;
        let mut warned = false;
        loop {
            if !enrolled(&user) {
                if available {
                    available = false;
                    if !send(Finger::Unavailable) {
                        return;
                    }
                }
                thread::sleep(UNAVAILABLE_PAUSE);
                continue;
            }
            if !available {
                available = true;
                if !send(Finger::Available) {
                    return;
                }
            }
            if first {
                first = false;
                thread::sleep(START_DELAY);
            }
            let started = Instant::now();
            let result = pam::authenticate(&service, &user, None, &mut |message| {
                if let Message::Error(text) = message {
                    if is_scan_feedback(&text) {
                        send(Finger::Retry(text));
                    }
                }
            });
            match result {
                Ok(()) => {
                    send(Finger::Matched);
                    return;
                }
                Err(err) if started.elapsed() < QUICK_FAILURE => {
                    pause = (pause * 2).min(MAX_RETRY_PAUSE);
                    if pause == MAX_RETRY_PAUSE && !warned {
                        warned = true;
                        eprintln!(
                            "tylock: fingerprint keeps failing (PAM service {service:?}: {err}); \
                             still retrying"
                        );
                    }
                    thread::sleep(pause);
                }
                // A timeout or used-up attempts: start again right away.
                Err(_) => {
                    pause = RETRY_PAUSE;
                    thread::sleep(RETRY_PAUSE);
                }
            }
        }
    });
}

const FPRINT: &str = "net.reactivated.Fprint";
const FPRINT_MANAGER_PATH: &str = "/net/reactivated/Fprint/Manager";
const FPRINT_MANAGER: &str = "net.reactivated.Fprint.Manager";
const FPRINT_DEVICE: &str = "net.reactivated.Fprint.Device";
/// fprintd is bus-activated, so the first call may start it.
const FPRINT_TIMEOUT: Duration = Duration::from_secs(5);

/// Where PAM looks for service files.
const PAM_DIRS: [&str; 3] = ["/etc/pam.d", "/usr/lib/pam.d", "/usr/share/pam.d"];

/// Whether PAM has a service called `service`. Without one PAM uses `other`, which
/// usually refuses everyone: locking then could never be undone.
pub fn pam_service_exists(service: &str) -> bool {
    !service.contains('/')
        && (PAM_DIRS
            .iter()
            .any(|dir| std::path::Path::new(dir).join(service).exists())
            || std::fs::read_to_string("/etc/pam.conf").is_ok_and(|conf| {
                conf.lines()
                    .any(|line| line.split_whitespace().next() == Some(service))
            }))
}

/// Whether `user` has a finger enrolled on the default reader, asked of fprintd. Without
/// fprintd, a reader or prints there is nothing to wait for.
fn enrolled(user: &str) -> bool {
    let Ok(conn) = Connection::new_system() else {
        return false;
    };
    let manager = conn.with_proxy(FPRINT, FPRINT_MANAGER_PATH, FPRINT_TIMEOUT);
    let device: Result<(dbus::Path<'static>,), _> =
        manager.method_call(FPRINT_MANAGER, "GetDefaultDevice", ());
    let Ok((path,)) = device else {
        return false;
    };
    let device = conn.with_proxy(FPRINT, path, FPRINT_TIMEOUT);
    let fingers: Result<(Vec<String>,), _> =
        device.method_call(FPRINT_DEVICE, "ListEnrolledFingers", (user,));
    matches!(fingers, Ok((fingers,)) if !fingers.is_empty())
}

/// The name of the user running tylock (the one who unlocks).
pub fn current_user() -> Option<String> {
    // SAFETY: getpwuid_r writes only within the buffers it is given; the name is copied
    // out before they go away.
    unsafe {
        let mut buf = vec![0 as libc::c_char; 4096];
        loop {
            let mut pwd = MaybeUninit::<libc::passwd>::uninit();
            let mut result: *mut libc::passwd = std::ptr::null_mut();
            let rc = libc::getpwuid_r(
                libc::getuid(),
                pwd.as_mut_ptr(),
                buf.as_mut_ptr(),
                buf.len(),
                &mut result,
            );
            if rc == libc::ERANGE && buf.len() < 1 << 20 {
                buf.resize(buf.len() * 2, 0);
                continue;
            }
            if rc != 0 || result.is_null() {
                return None;
            }
            let name = CStr::from_ptr(pwd.assume_init().pw_name);
            return Some(name.to_string_lossy().into_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_only_scan_feedback() {
        assert!(is_scan_feedback("Failed to match fingerprint"));
        assert!(is_scan_feedback("Swipe was too short, try again"));
        assert!(!is_scan_feedback("Verification timed out"));
        assert!(!is_scan_feedback("An unknown error occurred"));
    }
}
