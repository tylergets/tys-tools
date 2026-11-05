//! The screen as a whole: its size and a snapshot of what is on it.
//!
//! Coordinates are "screen pixels", relative to the top-left corner of the screen
//! ([`bounds`]): device pixels on X11, and the compositor's logical layout pixels on Wayland
//! (what `grim -g`, `wf-recorder -g` and the window manager use). [`logical_scale`] converts
//! them to GTK's logical pixels.

use std::io::Read;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use gtk::prelude::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

use crate::session;

/// A rectangle in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ScreenRect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Moved by `(dx, dy)`.
    pub fn offset(self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }
}

/// An RGB (8 bits per channel, no padding) snapshot of the whole screen (all monitors). On
/// Wayland it is taken at the highest monitor scale, so it can be larger than [`bounds`].
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// The whole screen. On X11 this is the root window at `(0, 0)`. On Wayland it is the box
/// around every monitor in the compositor's layout, which may start anywhere; everything else
/// in this crate is relative to its corner. On Wayland this asks GDK, so it must be called on
/// the GTK thread after `gtk::init`.
pub fn bounds() -> Result<ScreenRect> {
    if session::is_wayland() {
        wayland_bounds()
    } else {
        let (conn, screen_num) = x11_connect()?;
        let screen = &conn.setup().roots[screen_num];
        Ok(ScreenRect::new(
            0,
            0,
            u32::from(screen.width_in_pixels),
            u32::from(screen.height_in_pixels),
        ))
    }
}

/// Size of the whole screen in screen pixels.
pub fn size() -> Result<(u32, u32)> {
    bounds().map(|rect| (rect.width, rect.height))
}

/// Screen pixels per GTK logical pixel: the GDK scale factor on X11 (where screen pixels are
/// device pixels), and 1 on Wayland (where both are layout pixels).
pub fn logical_scale() -> f64 {
    if session::is_wayland() {
        return 1.0;
    }
    gtk::gdk::Display::default()
        .and_then(|display| display.monitor(0))
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1)
        .max(1)
        .into()
}

/// Snapshot the whole screen as it is displayed, including under a compositor. On X11 this
/// reads the root window like ffmpeg's x11grab; on Wayland it runs `grim`.
pub fn screenshot() -> Result<Screenshot> {
    if session::is_wayland() {
        grim_screenshot()
    } else {
        x11_screenshot()
    }
}

/// Each monitor's rectangle, in screen pixels relative to [`bounds`]. Must be called on the
/// GTK thread after `gtk::init`.
pub fn monitors() -> Result<Vec<ScreenRect>> {
    let display = gtk::gdk::Display::default().context("no display (is GTK initialized?)")?;
    let origin = bounds()?;
    // GDK reports logical pixels; on X11 screen pixels are device pixels.
    let scale = logical_scale();
    let scaled = |value: i32| (f64::from(value) * scale).round() as i32;
    Ok((0..display.n_monitors())
        .filter_map(|index| display.monitor(index))
        .map(|monitor| {
            let g = monitor.geometry();
            ScreenRect::new(
                scaled(g.x()) - origin.x,
                scaled(g.y()) - origin.y,
                scaled(g.width()).max(0) as u32,
                scaled(g.height()).max(0) as u32,
            )
        })
        .collect())
}

impl ScreenRect {
    pub fn contains(&self, (x, y): (i32, i32)) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width as i32
            && y < self.y + self.height as i32
    }

    /// The overlap with `other`, if any.
    pub fn intersect(&self, other: &ScreenRect) -> Option<ScreenRect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.width as i32).min(other.x + other.width as i32);
        let y1 = (self.y + self.height as i32).min(other.y + other.height as i32);
        (x1 > x0 && y1 > y0).then(|| ScreenRect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
    }
}

pub(crate) fn x11_connect() -> Result<(x11rb::rust_connection::RustConnection, usize)> {
    x11rb::connect(None).context("failed to connect to the X server; is DISPLAY set?")
}

fn x11_screenshot() -> Result<Screenshot> {
    let (conn, screen_num) = x11_connect()?;
    let screen = &conn.setup().roots[screen_num];
    let (width, height) = (screen.width_in_pixels, screen.height_in_pixels);
    let image = conn
        .get_image(ImageFormat::Z_PIXMAP, screen.root, 0, 0, width, height, !0)?
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
    Ok(Screenshot {
        width: u32::from(width),
        height: u32::from(height),
        rgb,
    })
}

fn wayland_bounds() -> Result<ScreenRect> {
    let display = gtk::gdk::Display::default().context("no display (is GTK initialized?)")?;
    let monitors: Vec<_> = (0..display.n_monitors())
        .filter_map(|index| display.monitor(index))
        .map(|monitor| monitor.geometry())
        .collect();
    if monitors.is_empty() {
        bail!("no monitors found");
    }
    let x0 = monitors.iter().map(|g| g.x()).min().unwrap_or(0);
    let y0 = monitors.iter().map(|g| g.y()).min().unwrap_or(0);
    let x1 = monitors
        .iter()
        .map(|g| g.x() + g.width())
        .max()
        .unwrap_or(0);
    let y1 = monitors
        .iter()
        .map(|g| g.y() + g.height())
        .max()
        .unwrap_or(0);
    Ok(ScreenRect::new(
        x0,
        y0,
        (x1 - x0).max(0) as u32,
        (y1 - y0).max(0) as u32,
    ))
}

fn grim_screenshot() -> Result<Screenshot> {
    let mut child = Command::new("grim")
        .args(["-t", "ppm", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run grim (needed for screenshots on Wayland)")?;
    let mut ppm = Vec::new();
    child
        .stdout
        .take()
        .context("no stdout from grim")?
        .read_to_end(&mut ppm)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "grim failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_ppm(ppm)
}

/// Parse a binary (P6), 8-bit PPM image.
fn parse_ppm(mut ppm: Vec<u8>) -> Result<Screenshot> {
    let mut fields = Vec::with_capacity(4);
    let mut pos = 0;
    while fields.len() < 4 {
        while ppm.get(pos).is_some_and(u8::is_ascii_whitespace) {
            pos += 1;
        }
        if ppm.get(pos) == Some(&b'#') {
            while ppm.get(pos).is_some_and(|&byte| byte != b'\n') {
                pos += 1;
            }
            continue;
        }
        let start = pos;
        while ppm.get(pos).is_some_and(|byte| !byte.is_ascii_whitespace()) {
            pos += 1;
        }
        if start == pos {
            bail!("truncated PPM header");
        }
        fields.push(String::from_utf8_lossy(&ppm[start..pos]).into_owned());
    }
    // Exactly one whitespace byte separates the header from the pixels.
    pos += 1;
    if fields[0] != "P6" || fields[3] != "255" {
        bail!("unsupported PPM format ({} max {})", fields[0], fields[3]);
    }
    let width: u32 = fields[1].parse().context("bad PPM width")?;
    let height: u32 = fields[2].parse().context("bad PPM height")?;
    let len = width as usize * height as usize * 3;
    if ppm.len() < pos + len {
        bail!("truncated PPM image");
    }
    ppm.drain(..pos);
    ppm.truncate(len);
    Ok(Screenshot {
        width,
        height,
        rgb: ppm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ppm() {
        let mut data = b"P6\n# comment\n2 1\n255\n".to_vec();
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let shot = parse_ppm(data).unwrap();
        assert_eq!((shot.width, shot.height), (2, 1));
        assert_eq!(shot.rgb, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn intersects_rects() {
        let a = ScreenRect::new(0, 0, 100, 100);
        let b = ScreenRect::new(50, 80, 100, 100);
        assert_eq!(a.intersect(&b), Some(ScreenRect::new(50, 80, 50, 20)));
        assert_eq!(a.intersect(&ScreenRect::new(100, 0, 10, 10)), None);
        assert!(a.contains((99, 0)) && !a.contains((100, 0)));
    }

    #[test]
    fn rejects_truncated_ppm() {
        assert!(parse_ppm(b"P6\n2 2\n255\n\x00\x00".to_vec()).is_err());
    }
}
