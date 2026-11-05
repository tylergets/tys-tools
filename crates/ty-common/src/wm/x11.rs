//! X11 windows: listing, geometry, click-to-pick and the pointer.

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, EventMask, GrabMode, GrabStatus, KeyButMask, MapState, Window,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

use super::{buttons, Pointer};
use crate::screen::{x11_connect, ScreenRect};

/// Glyph index of `XC_crosshair` in the standard X cursor font.
const XC_CROSSHAIR: u16 = 34;
const GRAB_RETRY_TIMEOUT: Duration = Duration::from_secs(1);
const LEFT_BUTTON: u8 = 1;

/// Visible top-level windows with their decorations, topmost first.
pub fn windows() -> Result<Vec<(Window, ScreenRect)>> {
    let (conn, screen_num) = x11_connect()?;
    let root = conn.setup().roots[screen_num].root;
    visible_windows(&conn, root)
}

/// Absolute on-screen geometry of a window (without decorations).
pub fn window_rect(window: Window) -> Result<ScreenRect> {
    let (conn, screen_num) = x11_connect()?;
    let root = conn.setup().roots[screen_num].root;
    let geometry = conn.get_geometry(window)?.reply()?;
    let origin = conn.translate_coordinates(window, root, 0, 0)?.reply()?;
    Ok(ScreenRect::new(
        i32::from(origin.dst_x),
        i32::from(origin.dst_y),
        u32::from(geometry.width),
        u32::from(geometry.height),
    ))
}

/// Polls the pointer position and buttons.
pub struct PointerPoller {
    conn: RustConnection,
    root: Window,
}

impl PointerPoller {
    pub fn new() -> Result<Self> {
        let (conn, screen_num) = x11_connect()?;
        let root = conn.setup().roots[screen_num].root;
        Ok(Self { conn, root })
    }

    pub fn poll(&self) -> Result<Pointer> {
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        Ok(Pointer {
            x: i32::from(pointer.root_x),
            y: i32::from(pointer.root_y),
            buttons: buttons_from_mask(pointer.mask),
        })
    }
}

fn buttons_from_mask(mask: KeyButMask) -> u8 {
    [
        (KeyButMask::BUTTON1, buttons::LEFT),
        (KeyButMask::BUTTON2, buttons::MIDDLE),
        (KeyButMask::BUTTON3, buttons::RIGHT),
    ]
    .into_iter()
    .filter(|(bit, _)| mask.contains(*bit))
    .fold(0, |buttons, (_, button)| buttons | button)
}

/// Show a crosshair and wait for the user to click a window.
///
/// Returns the client window that was clicked, or `None` if the user cancelled with any key
/// or a non-left mouse button.
pub fn select_window() -> Result<Option<Window>> {
    let (conn, screen_num) = x11_connect()?;
    let root = conn.setup().roots[screen_num].root;

    let font = conn.generate_id()?;
    conn.open_font(font, b"cursor")?;
    let cursor = conn.generate_id()?;
    conn.create_glyph_cursor(
        cursor,
        font,
        font,
        XC_CROSSHAIR,
        XC_CROSSHAIR + 1,
        0,
        0,
        0,
        0xffff,
        0xffff,
        0xffff,
    )?;

    grab_input(&conn, root, cursor)?;
    let selected = wait_for_click(&conn);

    conn.ungrab_pointer(CURRENT_TIME)?;
    conn.ungrab_keyboard(CURRENT_TIME)?;
    conn.free_cursor(cursor)?;
    conn.close_font(font)?;
    conn.flush()?;

    match selected? {
        Some(frame) => Ok(Some(find_client_window(&conn, frame)?.unwrap_or(frame))),
        None => Ok(None),
    }
}

/// Grab pointer and keyboard. Retries briefly because the tray menu may still hold a grab
/// right after the user clicks a menu item.
fn grab_input(conn: &RustConnection, root: Window, cursor: u32) -> Result<()> {
    let deadline = Instant::now() + GRAB_RETRY_TIMEOUT;
    loop {
        let pointer = conn
            .grab_pointer(
                false,
                root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                NONE,
                cursor,
                CURRENT_TIME,
            )?
            .reply()?
            .status;
        if pointer == GrabStatus::SUCCESS {
            let keyboard = conn
                .grab_keyboard(false, root, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)?
                .reply()?
                .status;
            if keyboard == GrabStatus::SUCCESS {
                return Ok(());
            }
            conn.ungrab_pointer(CURRENT_TIME)?;
        }

        if Instant::now() >= deadline {
            bail!("could not grab the pointer/keyboard to select a window");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_click(conn: &RustConnection) -> Result<Option<Window>> {
    let mut pressed: Option<Window> = None;
    loop {
        match conn.wait_for_event()? {
            Event::ButtonPress(event) if event.detail == LEFT_BUTTON => {
                if event.child != NONE {
                    pressed = Some(event.child);
                }
            }
            Event::ButtonPress(_) | Event::KeyPress(_) => return Ok(None),
            // Wait for release so the click is not delivered to the window after ungrabbing.
            Event::ButtonRelease(event) if event.detail == LEFT_BUTTON && pressed.is_some() => {
                return Ok(pressed);
            }
            _ => {}
        }
    }
}

/// Window managers reparent clients into frames; the click lands on the frame. Find the
/// descendant that carries `WM_STATE`, which is the actual client window.
fn find_client_window(conn: &RustConnection, window: Window) -> Result<Option<Window>> {
    let wm_state = conn.intern_atom(false, b"WM_STATE")?.reply()?.atom;
    find_window_with_property(conn, window, wm_state)
}

fn find_window_with_property(
    conn: &RustConnection,
    window: Window,
    property: u32,
) -> Result<Option<Window>> {
    let reply = conn
        .get_property(false, window, property, AtomEnum::ANY, 0, 0)?
        .reply()?;
    if reply.type_ != NONE {
        return Ok(Some(window));
    }

    for child in conn.query_tree(window)?.reply()?.children {
        if let Some(found) = find_window_with_property(conn, child, property)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// Visible top-level windows, topmost first. Uses the window manager's stacking list
/// (`_NET_CLIENT_LIST_STACKING`) when there is one, so the rectangles are client windows plus
/// their decorations (`_NET_FRAME_EXTENTS`); otherwise the root window's mapped children.
fn visible_windows(conn: &RustConnection, root: Window) -> Result<Vec<(Window, ScreenRect)>> {
    let stacking = intern(conn, "_NET_CLIENT_LIST_STACKING")?;
    let extents = intern(conn, "_NET_FRAME_EXTENTS")?;

    let clients = conn
        .get_property(false, root, stacking, AtomEnum::WINDOW, 0, u32::MAX)?
        .reply()?
        .value32()
        .map(|values| values.collect::<Vec<_>>())
        .unwrap_or_default();
    let (bottom_to_top, use_extents) = if clients.is_empty() {
        (conn.query_tree(root)?.reply()?.children, false)
    } else {
        (clients, true)
    };

    let mut windows = Vec::new();
    for &window in bottom_to_top.iter().rev() {
        if let Some(rect) = outer_rect(conn, root, window, use_extents.then_some(extents)) {
            windows.push((window, rect));
        }
    }
    Ok(windows)
}

/// The window's rectangle on the root window, or `None` if it isn't viewable (e.g. on
/// another workspace) or has no area.
fn outer_rect(
    conn: &RustConnection,
    root: Window,
    window: Window,
    frame_extents: Option<u32>,
) -> Option<ScreenRect> {
    let attributes = conn.get_window_attributes(window).ok()?.reply().ok()?;
    if attributes.map_state != MapState::VIEWABLE {
        return None;
    }
    let geometry = conn.get_geometry(window).ok()?.reply().ok()?;
    let origin = conn
        .translate_coordinates(window, root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    // The geometry excludes the window's own X border; include it, as it is drawn too.
    let border = i32::from(geometry.border_width);
    let (mut left, mut right, mut top, mut bottom) = (border, border, border, border);
    if let Some(atom) = frame_extents {
        let extents: Vec<u32> = conn
            .get_property(false, window, atom, AtomEnum::CARDINAL, 0, 4)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().map(|values| values.collect()))
            .unwrap_or_default();
        if let [l, r, t, b] = extents[..] {
            let clamp = |value: u32| value.min(1000) as i32;
            (left, right, top, bottom) = (
                left + clamp(l),
                right + clamp(r),
                top + clamp(t),
                bottom + clamp(b),
            );
        }
    }
    let width = i32::from(geometry.width) + left + right;
    let height = i32::from(geometry.height) + top + bottom;
    if width <= 0 || height <= 0 {
        return None;
    }
    // `translate_coordinates` gives the inside of the X border; step out past it and the
    // decorations.
    Some(ScreenRect::new(
        i32::from(origin.dst_x) - left,
        i32::from(origin.dst_y) - top,
        width as u32,
        height as u32,
    ))
}

fn intern(conn: &RustConnection, name: &str) -> Result<u32> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_button_mask() {
        assert_eq!(buttons_from_mask(KeyButMask::BUTTON1), buttons::LEFT);
        assert_eq!(
            buttons_from_mask(KeyButMask::BUTTON3 | KeyButMask::SHIFT),
            buttons::RIGHT
        );
    }
}
