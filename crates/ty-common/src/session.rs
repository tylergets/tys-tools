//! Which display server and window manager the tools are running under.
//!
//! Wayland wins when both `WAYLAND_DISPLAY` and `DISPLAY` are set (XWayland), matching GTK,
//! which also picks its Wayland backend in that case.

use std::env;

use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    X11,
    Wayland,
}

/// The compositor or window manager, as far as the tools care about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compositor {
    /// sway (speaks the i3 IPC protocol).
    Sway,
    Hyprland,
    /// Any other compositor or X11 window manager. On X11 this includes i3, which
    /// [`crate::wm`] finds through its IPC socket rather than through the environment.
    Other,
}

/// The current session, or `None` outside a graphical session.
pub fn current() -> Option<Session> {
    if is_set("WAYLAND_DISPLAY") {
        Some(Session::Wayland)
    } else if is_set("DISPLAY") {
        Some(Session::X11)
    } else {
        None
    }
}

/// The current session, or an error naming `tool` outside a graphical session.
pub fn require(tool: &str) -> Result<Session> {
    match current() {
        Some(session) => Ok(session),
        None => {
            bail!("neither WAYLAND_DISPLAY nor DISPLAY is set; {tool} needs a graphical session")
        }
    }
}

pub fn is_wayland() -> bool {
    current() == Some(Session::Wayland)
}

/// The running Wayland compositor. Always [`Compositor::Other`] on X11.
pub fn compositor() -> Compositor {
    if !is_wayland() {
        Compositor::Other
    } else if is_set("HYPRLAND_INSTANCE_SIGNATURE") {
        Compositor::Hyprland
    } else if is_set("SWAYSOCK") {
        Compositor::Sway
    } else {
        Compositor::Other
    }
}

fn is_set(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| !value.is_empty())
}
