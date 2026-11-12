//! Locking the screen: covering every monitor and taking all keyboard input until the caller
//! decides to unlock. What is drawn and how the password is checked is up to the caller (a
//! [`Handler`]); this module only owns the screens and the input.
//!
//! On Wayland it uses `ext-session-lock-v1` (sway, Hyprland, river, niri, KDE, ...): the
//! compositor itself keeps the session locked, also if the locker crashes. Fractional
//! scaling is honored, so drawing happens at the monitor's real resolution.
//!
//! On X11 it is one override-redirect window over the whole root window that grabs the
//! keyboard and pointer, raises itself when something covers it, and follows RandR changes.
//! Each monitor is its own surface there too.
//!
//! Either way the handler draws every surface at its physical pixel size, so the same
//! layout looks the same on a 1080p and a 4K monitor if it is sized relative to the surface.

#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;

use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};

use crate::session::{self, Session};

/// One locked monitor, as given to [`Handler::draw`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    /// Stable for as long as the monitor is connected.
    pub id: u32,
    /// Physical pixels.
    pub width: u32,
    pub height: u32,
}

/// Pixels to draw one output into: 32 bits per pixel, `0xXXRRGGBB` in native byte order
/// (cairo's `RGB24`, `wl_shm`'s `XRGB8888`). Starts with the previous frame's content or
/// garbage, so draw every pixel.
pub struct Canvas<'a> {
    pub data: &'a mut [u8],
    pub width: u32,
    pub height: u32,
    /// Bytes per row.
    pub stride: u32,
}

/// The part of a [`Canvas`] a draw changed, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Damage {
    /// Clipped to a `width`×`height` canvas; `None` if nothing is left.
    fn clip(self, width: u32, height: u32) -> Option<Self> {
        let x = self.x.min(width);
        let y = self.y.min(height);
        let right = self.x.saturating_add(self.width).min(width);
        let bottom = self.y.saturating_add(self.height).min(height);
        (right > x && bottom > y).then_some(Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Enter,
    Backspace,
    Escape,
    /// What the key types, already through the keyboard layout and modifiers.
    Text(String),
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub ctrl: bool,
}

/// What the handler wants after an event; see [`Handler::control`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Control {
    /// Draw every output again.
    pub redraw: bool,
    /// Unlock the screen and return from [`run`].
    pub unlock: bool,
    /// Ask again at this time even without input (a clock, a message that times out).
    pub wake_at: Option<Instant>,
}

/// The locker's side: drawing and reacting to keys.
pub trait Handler {
    /// Draw one output. Called when it appears or changes size, and for every output when
    /// [`Control::redraw`] asks for it. Draw the whole canvas, but return the part that
    /// changed since the previous draw of this output, if not everything: only that is
    /// sent to the display (which keeps animations smooth on large monitors). The canvas
    /// may hold an older frame than the previous one, so the damage is only a hint.
    fn draw(&mut self, output: &Output, canvas: Canvas<'_>) -> Option<Damage>;

    /// A key was pressed (repeats included).
    fn key(&mut self, event: KeyEvent);

    /// Caps Lock turned on or off (also called once with the initial state).
    fn caps_lock(&mut self, _on: bool) {}

    /// Every monitor is covered and input is taken: the screen is locked.
    fn locked(&mut self) {}

    /// Asked after every batch of events, wake-up and timeout.
    fn control(&mut self) -> Control;
}

/// Wakes [`run`]'s loop from another thread, so [`Handler::control`] is asked again. Cheap
/// to clone.
#[derive(Clone)]
pub struct Waker {
    fd: Arc<OwnedFd>,
}

impl Waker {
    pub fn wake(&self) {
        let byte = 1u8;
        // SAFETY: writes one byte from a valid buffer to our own pipe. A full pipe means a
        // wake-up is already pending, so the result doesn't matter.
        unsafe {
            libc::write(self.fd.as_raw_fd(), (&byte as *const u8).cast(), 1);
        }
    }
}

/// The receiving end of a [`Waker`], handed to [`run`].
pub struct WakeSource {
    fd: OwnedFd,
}

impl WakeSource {
    /// Read every pending wake-up.
    fn drain(&self) {
        let mut buf = [0u8; 64];
        // SAFETY: reads into a valid buffer from our own non-blocking pipe.
        while unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
    }

    fn raw(&self) -> i32 {
        self.fd.as_fd().as_raw_fd()
    }
}

pub fn waker() -> Result<(Waker, WakeSource)> {
    let mut fds = [0; 2];
    // SAFETY: pipe2 fills both descriptors on success.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error()).context("failed to create a pipe");
    }
    // SAFETY: both descriptors are new and owned by nobody else.
    let (read, write) = unsafe {
        use std::os::fd::FromRawFd;
        (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))
    };
    Ok((
        Waker {
            fd: Arc::new(write),
        },
        WakeSource { fd: read },
    ))
}

/// Lock the screen and run until the handler unlocks it. Fails without locking when the
/// screen can't be locked (another locker holds it, no session-lock support, the input
/// can't be grabbed). On Wayland an error after locking leaves the session locked.
pub fn run(handler: &mut dyn Handler, wake: WakeSource) -> Result<()> {
    match session::current() {
        Some(Session::Wayland) => wayland::run(handler, wake),
        Some(Session::X11) => x11::run(handler, wake),
        None => bail!("neither WAYLAND_DISPLAY nor DISPLAY is set; nothing to lock"),
    }
}

/// Wait until `fds` are readable, the wake pipe is written to, or `deadline` passes.
/// Returns which of `fds` are readable.
fn wait(fds: &[i32], wake: &WakeSource, deadline: Option<Instant>) -> Result<Vec<bool>> {
    let mut polls: Vec<libc::pollfd> = fds
        .iter()
        .chain(std::iter::once(&wake.raw()))
        .map(|&fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let timeout = deadline.map_or(-1, |deadline| {
        let left = deadline.saturating_duration_since(Instant::now());
        // Round up, so we don't wake just before the deadline and spin.
        i32::try_from(left.as_micros().div_ceil(1000)).unwrap_or(i32::MAX)
    });
    // SAFETY: `polls` is a valid array of pollfd for the duration of the call.
    let n = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, timeout) };
    if n < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err).context("poll failed");
        }
    }
    wake.drain();
    Ok(polls[..fds.len()]
        .iter()
        .map(|p| p.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0)
        .collect())
}

/// The earlier of two optional deadlines.
fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn waker_wakes_and_drains() {
        let (waker, source) = waker().unwrap();
        waker.wake();
        waker.wake();
        let start = Instant::now();
        wait(&[], &source, Some(start + Duration::from_secs(5))).unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        // Drained: the next wait times out.
        let start = Instant::now();
        wait(&[], &source, Some(start + Duration::from_millis(20))).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(20));
    }

    #[test]
    fn damage_clips() {
        let d = Damage {
            x: 10,
            y: 90,
            width: 50,
            height: 50,
        };
        assert_eq!(
            d.clip(100, 100),
            Some(Damage {
                x: 10,
                y: 90,
                width: 50,
                height: 10
            })
        );
        assert_eq!(d.clip(5, 100), None);
    }

    #[test]
    fn earliest_deadline() {
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        assert_eq!(earliest(Some(later), Some(now)), Some(now));
        assert_eq!(earliest(None, Some(now)), Some(now));
        assert_eq!(earliest(None, None), None);
    }
}
