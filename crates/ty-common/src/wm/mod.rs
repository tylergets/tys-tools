//! Windows and the window manager: listing visible windows, window geometry, the focused
//! window, moving a window aside for recording, and the pointer.
//!
//! Backends: X11 (any EWMH window manager, plus i3 over its IPC socket), sway (i3 IPC) and
//! Hyprland (its request socket). Other Wayland compositors don't expose windows to clients.
//! Rectangles and positions are in screen pixels relative to [`crate::screen::bounds`].

pub mod hyprland;
pub mod ipc;
pub mod x11;

use anyhow::{bail, Context, Result};

pub use ipc::WindowPlacement;

use crate::input::PointerButtons;
use crate::screen::{self, ScreenRect};
use crate::session::{self, Compositor};

/// Bits of [`Pointer::buttons`].
pub mod buttons {
    pub const LEFT: u8 = 1;
    pub const MIDDLE: u8 = 2;
    pub const RIGHT: u8 = 4;
}

/// A window, as its window manager knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowId {
    /// An X11 window id (X11 sessions, including i3).
    X11(u32),
    /// A sway container id.
    Sway(i64),
    /// A Hyprland window address (`0x...`).
    Hyprland(String),
}

/// A visible window and its outer rectangle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    pub rect: ScreenRect,
}

/// The focused window, as reported by the window manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveWindow {
    pub title: String,
    pub app_id: Option<String>,
    pub pid: Option<i64>,
    pub window_id: Option<String>,
    pub workspace_name: Option<String>,
    pub is_xwayland: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pointer {
    pub x: i32,
    pub y: i32,
    /// Bitmask of [`buttons`].
    pub buttons: u8,
}

/// Visible windows (with decorations where known), topmost first. On Wayland this calls
/// [`screen::bounds`], so it must run on the GTK thread.
pub fn visible_windows() -> Result<Vec<Window>> {
    if !session::is_wayland() {
        return Ok(x11::windows()?
            .into_iter()
            .map(|(id, rect)| Window {
                id: WindowId::X11(id),
                rect,
            })
            .collect());
    }
    let windows = match session::compositor() {
        Compositor::Sway => ipc::visible_windows()?,
        Compositor::Hyprland => hyprland::visible_windows()?,
        Compositor::Other => bail!("this compositor does not list its windows"),
    };
    let (dx, dy) = origin()?;
    Ok(windows
        .into_iter()
        .map(|window| Window {
            rect: window.rect.offset(-dx, -dy),
            ..window
        })
        .collect())
}

/// Current geometry of a window. On Wayland this must run on the GTK thread.
pub fn window_rect(window: &WindowId) -> Result<ScreenRect> {
    let rect = match window {
        WindowId::X11(id) if !session::is_wayland() => return x11::window_rect(*id),
        WindowId::X11(_) | WindowId::Sway(_) => ipc::window_rect(window)?,
        WindowId::Hyprland(address) => hyprland::window_rect(address)?,
    };
    let (dx, dy) = origin()?;
    Ok(rect.offset(-dx, -dy))
}

/// X11 only: show a crosshair and wait for the user to click a window. `None` if cancelled.
/// Blocks; run it off the GTK thread.
pub fn pick_x11_window() -> Result<Option<WindowId>> {
    Ok(x11::select_window()?.map(WindowId::X11))
}

/// Whether the window manager can move windows aside for recording (i3 or sway).
pub fn can_stage_windows() -> bool {
    ipc::available()
}

/// Move `window` to `workspace` as a borderless floating window of the given size, and switch
/// there. Returns where it was, for [`restore_window`]. Needs i3 or sway.
pub fn stage_window(
    window: &WindowId,
    width: u32,
    height: u32,
    workspace: &str,
) -> Result<Option<WindowPlacement>> {
    ipc::stage_window_for_recording(window, width, height, workspace)
}

/// Put a window back where [`stage_window`] found it.
pub fn restore_window(placement: &WindowPlacement) -> Result<()> {
    ipc::restore_window(placement)
}

/// The monitor with keyboard focus (sway, Hyprland), else the first one; relative to
/// [`screen::bounds`]. Must run on the GTK thread.
pub fn focused_monitor() -> Result<ScreenRect> {
    let focused = match session::compositor() {
        Compositor::Sway => ipc::IpcConnection::connect()?.focused_output()?,
        Compositor::Hyprland => hyprland::focused_monitor()?,
        Compositor::Other => None,
    };
    if let Some(rect) = focused {
        let (dx, dy) = origin()?;
        return Ok(rect.offset(-dx, -dy));
    }
    screen::monitors()?
        .into_iter()
        .next()
        .context("no monitors found")
}

/// The focused window, via i3/sway IPC. `None` elsewhere.
pub fn active_window() -> Option<ActiveWindow> {
    ipc::available().then(ipc::active_window).flatten()
}

/// Polls the pointer. Supported on X11 and Hyprland; on Hyprland, buttons come from
/// `/dev/input` (see [`PointerButtons`]) and read as 0 without access to it.
pub struct PointerPoller {
    backend: PointerBackend,
}

enum PointerBackend {
    X11(Box<x11::PointerPoller>),
    Hyprland {
        origin: (i32, i32),
        buttons: Option<PointerButtons>,
    },
}

impl PointerPoller {
    /// On Wayland this calls [`screen::bounds`], so it must run on the GTK thread; the poller
    /// itself can then move to another thread.
    pub fn new() -> Result<Self> {
        let backend = if !session::is_wayland() {
            PointerBackend::X11(Box::new(x11::PointerPoller::new()?))
        } else if session::compositor() == Compositor::Hyprland {
            let buttons = PointerButtons::watch()
                .map_err(|err| eprintln!("warning: clicks will not be logged: {err:#}"))
                .ok();
            PointerBackend::Hyprland {
                origin: origin()?,
                buttons,
            }
        } else {
            bail!("this compositor does not report the pointer position (X11 and Hyprland do)");
        };
        Ok(Self { backend })
    }

    pub fn poll(&self) -> Result<Pointer> {
        match &self.backend {
            PointerBackend::X11(poller) => poller.poll(),
            PointerBackend::Hyprland { origin, buttons } => {
                let (x, y) = hyprland::cursor_position()?;
                Ok(Pointer {
                    x: x - origin.0,
                    y: y - origin.1,
                    buttons: buttons.as_ref().map_or(0, PointerButtons::state),
                })
            }
        }
    }
}

/// Where the layout's screen-pixel origin is (always `(0, 0)` on X11).
fn origin() -> Result<(i32, i32)> {
    if !session::is_wayland() {
        return Ok((0, 0));
    }
    let bounds = screen::bounds()?;
    Ok((bounds.x, bounds.y))
}
