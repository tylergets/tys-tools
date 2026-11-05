//! Keyboard input and output: typing into the focused window, and listening to every key
//! press system-wide.
//!
//! - Typing: XTest via `enigo` on X11; `wtype` (the virtual-keyboard protocol, supported by
//!   wlroots compositors such as sway and Hyprland) on Wayland.
//! - Listening: XRecord via `rdev` on X11. Wayland doesn't let one client see another's
//!   keys, so there we read the keyboards in `/dev/input` directly (this needs read access to
//!   them, usually by being in the `input` group) and translate keys with xkbcommon, using
//!   the `XKB_DEFAULT_*` environment variables for the layout.

mod evdev;
mod x11;

use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use enigo::{Direction, Enigo, Key as EnigoKey, Keyboard as _, Settings};

pub use evdev::PointerButtons;

use crate::session;

/// A key that can be typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Tab,
    Escape,
    Backspace,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Delete,
    PageUp,
    PageDown,
    /// F1 to F12.
    F(u8),
}

impl Key {
    fn enigo(self) -> Result<EnigoKey> {
        Ok(match self {
            Key::Enter => EnigoKey::Return,
            Key::Tab => EnigoKey::Tab,
            Key::Escape => EnigoKey::Escape,
            Key::Backspace => EnigoKey::Backspace,
            Key::Space => EnigoKey::Space,
            Key::Left => EnigoKey::LeftArrow,
            Key::Right => EnigoKey::RightArrow,
            Key::Up => EnigoKey::UpArrow,
            Key::Down => EnigoKey::DownArrow,
            Key::Home => EnigoKey::Home,
            Key::End => EnigoKey::End,
            Key::Delete => EnigoKey::Delete,
            Key::PageUp => EnigoKey::PageUp,
            Key::PageDown => EnigoKey::PageDown,
            Key::F(1) => EnigoKey::F1,
            Key::F(2) => EnigoKey::F2,
            Key::F(3) => EnigoKey::F3,
            Key::F(4) => EnigoKey::F4,
            Key::F(5) => EnigoKey::F5,
            Key::F(6) => EnigoKey::F6,
            Key::F(7) => EnigoKey::F7,
            Key::F(8) => EnigoKey::F8,
            Key::F(9) => EnigoKey::F9,
            Key::F(10) => EnigoKey::F10,
            Key::F(11) => EnigoKey::F11,
            Key::F(12) => EnigoKey::F12,
            Key::F(n) => bail!("no such key: F{n}"),
        })
    }

    /// The X keysym name, as `wtype -k` takes it.
    fn keysym_name(self) -> Result<String> {
        Ok(match self {
            Key::Enter => "Return".into(),
            Key::Tab => "Tab".into(),
            Key::Escape => "Escape".into(),
            Key::Backspace => "BackSpace".into(),
            Key::Space => "space".into(),
            Key::Left => "Left".into(),
            Key::Right => "Right".into(),
            Key::Up => "Up".into(),
            Key::Down => "Down".into(),
            Key::Home => "Home".into(),
            Key::End => "End".into(),
            Key::Delete => "Delete".into(),
            Key::PageUp => "Prior".into(),
            Key::PageDown => "Next".into(),
            Key::F(n @ 1..=12) => format!("F{n}"),
            Key::F(n) => bail!("no such key: F{n}"),
        })
    }
}

/// Types into whichever window has keyboard focus.
pub struct Keyboard {
    backend: Backend,
}

enum Backend {
    X11(Mutex<Enigo>),
    Wayland { delay_ms: u32 },
}

impl Keyboard {
    /// `delay_ms` is the pause between key events (the backend's default if `None`).
    pub fn new(delay_ms: Option<u32>) -> Result<Self> {
        let backend = if session::is_wayland() {
            Backend::Wayland {
                delay_ms: delay_ms.unwrap_or(0),
            }
        } else {
            let mut settings = Settings::default();
            if let Some(delay) = delay_ms {
                settings.linux_delay = delay;
            }
            let enigo = Enigo::new(&settings)
                .map_err(|err| anyhow!("failed to set up keyboard input: {err}"))?;
            Backend::X11(Mutex::new(enigo))
        };
        Ok(Self { backend })
    }

    /// Type `text`. Blocks until it has been typed.
    pub fn type_text(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        match &self.backend {
            Backend::X11(enigo) => {
                lock(enigo)
                    .text(text)
                    .map_err(|err| anyhow!("typing failed: {err}"))?;
                std::thread::sleep(Duration::from_millis(1));
                Ok(())
            }
            Backend::Wayland { delay_ms } => wtype(*delay_ms, ["--".to_string(), text.to_string()]),
        }
    }

    /// Press and release `key`, `count` times.
    pub fn tap(&self, key: Key, count: usize) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        match &self.backend {
            Backend::X11(enigo) => {
                let key = key.enigo()?;
                let mut enigo = lock(enigo);
                for _ in 0..count {
                    for direction in [Direction::Press, Direction::Release] {
                        enigo
                            .key(key, direction)
                            .map_err(|err| anyhow!("key simulation failed: {err}"))?;
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                Ok(())
            }
            Backend::Wayland { delay_ms } => {
                let name = key.keysym_name()?;
                let args = (0..count).flat_map(|_| ["-k".to_string(), name.clone()]);
                wtype(*delay_ms, args)
            }
        }
    }
}

/// Type `text` into the focused window once, waiting `delay_ms` between keys, ignoring any
/// modifiers the user still holds (e.g. from the hotkey that triggered it). Uses
/// `xdotool type --clearmodifiers` on X11; on Wayland, `wtype`'s virtual keyboard has its own
/// modifier state, so held modifiers don't apply anyway. Blocks until typing finishes.
pub fn type_text(text: &str, delay_ms: u32) -> Result<()> {
    if session::is_wayland() {
        return wtype(delay_ms, ["--".to_string(), text.to_string()]);
    }
    let output = Command::new("xdotool")
        .args(["type", "--clearmodifiers", "--delay"])
        .arg(delay_ms.to_string())
        .arg("--")
        .arg(text)
        .output()
        .context("failed to run xdotool")?;
    if !output.status.success() {
        bail!(
            "xdotool failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn lock(enigo: &Mutex<Enigo>) -> std::sync::MutexGuard<'_, Enigo> {
    enigo
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wtype(delay_ms: u32, args: impl IntoIterator<Item = String>) -> Result<()> {
    let output = Command::new("wtype")
        .arg("-d")
        .arg(delay_ms.to_string())
        .args(args)
        .output()
        .context("failed to run wtype (needed to type on Wayland)")?;
    if !output.status.success() {
        bail!(
            "wtype failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// A key, as seen by [`listen`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKey {
    Shift,
    Ctrl,
    Alt,
    Meta,
    CapsLock,
    Enter,
    Tab,
    Backspace,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Delete,
    PageUp,
    PageDown,
    /// F1 to F12.
    F(u8),
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInput {
    /// Press (or auto-repeat) rather than release.
    pub pressed: bool,
    pub key: InputKey,
    /// The character the press produces with the current modifiers, if exactly one. Control
    /// characters are included (e.g. `'\r'` for Enter), as on X11.
    pub text: Option<char>,
}

/// Call `on_key` for every key press and release anywhere in the session. Blocks for as
/// long as it listens.
pub fn listen(on_key: impl FnMut(KeyInput) + Send + 'static) -> Result<()> {
    if session::is_wayland() {
        evdev::listen(on_key)
    } else {
        x11::listen(on_key)
    }
}

/// The only character in `s`, if it has exactly one.
fn single_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keysyms_for_wtype() {
        assert_eq!(Key::Backspace.keysym_name().unwrap(), "BackSpace");
        assert_eq!(Key::PageDown.keysym_name().unwrap(), "Next");
        assert_eq!(Key::F(12).keysym_name().unwrap(), "F12");
        assert!(Key::F(13).keysym_name().is_err());
    }

    #[test]
    fn extracts_single_char() {
        assert_eq!(single_char("a"), Some('a'));
        assert_eq!(single_char("ab"), None);
        assert_eq!(single_char(""), None);
    }
}
