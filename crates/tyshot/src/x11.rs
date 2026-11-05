use anyhow::{bail, Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, ImageFormat, MapState, Window};
use x11rb::rust_connection::RustConnection;

/// An RGB (8 bits per channel, no padding) snapshot of the whole screen, plus the windows
/// that were visible when it was taken.
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    /// Visible top-level windows, topmost first, in device pixels on the screen (may extend
    /// past its edges).
    pub windows: Vec<WindowRect>,
}

/// A window's outer rectangle (including window-manager decorations), in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Snapshot the whole X screen (all monitors) as it is displayed. `GetImage` on the root
/// window returns the composited result under a compositor too.
pub fn screenshot() -> Result<Screenshot> {
    let (conn, screen_num) =
        x11rb::connect(None).context("failed to connect to the X server; is DISPLAY set?")?;
    let screen = &conn.setup().roots[screen_num];
    let (root, width, height) = (screen.root, screen.width_in_pixels, screen.height_in_pixels);
    let image = conn
        .get_image(ImageFormat::Z_PIXMAP, root, 0, 0, width, height, !0)?
        .reply()?;

    let pixels = usize::from(width) * usize::from(height);
    if image.depth < 24 || image.data.len() < pixels * 4 {
        bail!("unsupported screen format (depth {})", image.depth);
    }
    // 24/32-bit TrueColor is BGRX in memory on little-endian servers.
    let mut rgb = Vec::with_capacity(pixels * 3);
    for bgrx in image.data.chunks_exact(4).take(pixels) {
        rgb.extend_from_slice(&[bgrx[2], bgrx[1], bgrx[0]]);
    }

    // Window snapping is a convenience: without it, selection still works by dragging.
    let windows = visible_windows(&conn, root).unwrap_or_else(|err| {
        eprintln!("warning: could not list windows for snapping: {err:#}");
        Vec::new()
    });
    Ok(Screenshot {
        width: u32::from(width),
        height: u32::from(height),
        rgb,
        windows,
    })
}

/// Visible top-level windows, topmost first. Uses the window manager's stacking list
/// (`_NET_CLIENT_LIST_STACKING`) when there is one, so the rectangles are client windows plus
/// their decorations (`_NET_FRAME_EXTENTS`); otherwise the root window's mapped children.
fn visible_windows(conn: &RustConnection, root: Window) -> Result<Vec<WindowRect>> {
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
        if let Some(rect) = window_rect(conn, root, window, use_extents.then_some(extents)) {
            windows.push(rect);
        }
    }
    Ok(windows)
}

/// The window's rectangle on the root window, or `None` if it isn't viewable (e.g. on
/// another workspace) or has no area.
fn window_rect(
    conn: &RustConnection,
    root: Window,
    window: Window,
    frame_extents: Option<u32>,
) -> Option<WindowRect> {
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
    Some(WindowRect {
        x: i32::from(origin.dst_x) - left,
        y: i32::from(origin.dst_y) - top,
        width: width as u32,
        height: height as u32,
    })
}

fn intern(conn: &RustConnection, name: &str) -> Result<u32> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}
