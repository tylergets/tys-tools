//! The X11 side of [`super::run`]: one override-redirect window over the root window, with
//! the keyboard and pointer grabbed. Each RandR monitor is an output; they are drawn into a
//! pixmap that is the window's background, so the server repaints exposures by itself.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::shm::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    self, ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask,
    GrabMode, GrabStatus, ImageFormat, ImageOrder, KeyButMask, StackMode, Visibility, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE};

use super::{Canvas, Damage, Handler, Key, KeyEvent, Output, WakeSource};
use crate::input::x11::Keymap;

/// How long to keep trying to grab input: a window manager holds the keyboard until the
/// key of the binding that ran the locker is released, and menus hold it while open.
const GRAB_ATTEMPTS: u32 = 150;
const GRAB_RETRY: Duration = Duration::from_millis(20);

// Keysyms (X11/keysymdef.h).
const RETURN: u32 = 0xff0d;
const KP_ENTER: u32 = 0xff8d;
const BACKSPACE: u32 = 0xff08;
const ESCAPE: u32 = 0xff1b;

pub fn run(handler: &mut dyn Handler, wake: WakeSource) -> Result<()> {
    let (conn, screen_num) =
        x11rb::connect(None).context("failed to connect to the X server; is DISPLAY set?")?;
    let screen = conn.setup().roots[screen_num].clone();
    let depth = screen.root_depth;
    let bpp = conn
        .setup()
        .pixmap_formats
        .iter()
        .find(|format| format.depth == depth)
        .map(|format| format.bits_per_pixel);
    if depth < 24 || bpp != Some(32) || conn.setup().image_byte_order != ImageOrder::LSB_FIRST {
        bail!("unsupported X screen format (depth {depth}, {bpp:?} bits per pixel)");
    }
    let root = screen.root;
    // Compositors can show ARGB notifications without a VisibilityNotify on the
    // lock window. Watch root children as well, including override-redirect
    // popups (Dunst, tynotif, menus, and OSDs). SUBSTRUCTURE_NOTIFY observes them
    // without taking over the window manager's SUBSTRUCTURE_REDIRECT selection.
    conn.change_window_attributes(
        root,
        &xproto::ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
    )?
    .check()?;
    let has_randr = conn
        .extension_information(randr::X11_EXTENSION_NAME)?
        .is_some();

    let cursor = invisible_cursor(&conn, root)?;
    let window = conn.generate_id()?;
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        window,
        root,
        0,
        0,
        screen.width_in_pixels,
        screen.height_in_pixels,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(screen.black_pixel)
            .cursor(cursor)
            .event_mask(
                EventMask::KEY_PRESS
                    | EventMask::KEY_RELEASE
                    | EventMask::VISIBILITY_CHANGE
                    | EventMask::STRUCTURE_NOTIFY,
            ),
    )?;
    conn.change_property8(
        xproto::PropMode::REPLACE,
        window,
        xproto::AtomEnum::WM_NAME,
        xproto::AtomEnum::STRING,
        b"tylock",
    )?;
    if has_randr {
        conn.randr_select_input(root, randr::NotifyMask::SCREEN_CHANGE)?;
    }
    let mut keymap = Keymap::fetch(&conn)?;
    let mut caps = conn
        .query_pointer(root)?
        .reply()?
        .mask
        .contains(KeyButMask::LOCK);
    handler.caps_lock(caps);

    // Draw before mapping, so the window appears with its content rather than black.
    let mut canvas = Screen::new(&conn, window, root, depth, has_randr)?;
    canvas.draw(&conn, handler, true)?;
    // Whether the server is done with the last frame: a round trip queued behind it.
    let mut frame_done = Some(conn.get_input_focus()?);
    conn.map_window(window)?;
    raise(&conn, window)?;
    conn.flush()?;

    if let Err(err) = grab(&conn, window, cursor) {
        let _ = conn.destroy_window(window);
        let _ = conn.flush();
        return Err(err);
    }
    handler.locked();

    let fd = conn.stream().as_raw_fd();
    let mut pending: Option<Event> = None;
    loop {
        let mut events: Vec<Event> = pending.take().into_iter().collect();
        while let Some(event) = conn.poll_for_event()? {
            events.push(event);
        }
        let mut rebuild = false;
        let mut restack = false;
        for event in events {
            match event {
                Event::KeyPress(press) => {
                    if let Some(key) = key_event(&keymap, press.detail, press.state) {
                        handler.key(key);
                    }
                    let now = press.state.contains(KeyButMask::LOCK);
                    if now != caps {
                        caps = now;
                        handler.caps_lock(caps);
                    }
                }
                // The state at a release already includes a Caps Lock toggled by its press.
                Event::KeyRelease(release) => {
                    let now = release.state.contains(KeyButMask::LOCK);
                    if now != caps {
                        caps = now;
                        handler.caps_lock(caps);
                    }
                }
                Event::MappingNotify(_) => keymap = Keymap::fetch(&conn)?,
                Event::VisibilityNotify(notify) if notify.state != Visibility::UNOBSCURED => {
                    restack = true;
                }
                Event::MapNotify(notify) if notify.event == root && notify.window != window => {
                    restack = true;
                }
                Event::ConfigureNotify(notify)
                    if notify.event == root && notify.window != window =>
                {
                    restack = true;
                }
                Event::CirculateNotify(notify)
                    if notify.event == root && notify.window != window =>
                {
                    restack = true;
                }
                Event::RandrScreenChangeNotify(_) => rebuild = true,
                Event::Error(err) => eprintln!("warning: X11 error: {err:?}"),
                _ => {}
            }
        }
        // Ignore our own configure notifications above to avoid a restacking
        // loop, and raise only once for a batch of other windows' changes.
        if restack {
            raise(&conn, window)?;
        }
        if rebuild {
            canvas.rebuild(&conn)?;
        }

        let control = handler.control();
        if control.unlock {
            let _ = conn.ungrab_keyboard(CURRENT_TIME);
            let _ = conn.ungrab_pointer(CURRENT_TIME);
            let _ = conn.destroy_window(window);
            canvas.free(&conn);
            let _ = conn.free_cursor(cursor);
            conn.flush()?;
            return Ok(());
        }
        let redraw = control.redraw || rebuild || canvas.monitors.iter().any(|m| m.dirty);
        if redraw {
            // Never draw faster than the server shows frames: a slow server (a GPU
            // pixmap, a big monitor) would queue them up and lag behind the keys. Keys
            // typed meanwhile all land in the next frame.
            if let Some(done) = frame_done.take() {
                done.reply()?;
            }
            if canvas.draw(&conn, handler, control.redraw || rebuild)? {
                frame_done = Some(conn.get_input_focus()?);
            }
        }
        conn.flush()?;

        // Events read while writing would not wake poll.
        if let Some(event) = conn.poll_for_event()? {
            pending = Some(event);
            continue;
        }
        super::wait(&[fd], &wake, control.wake_at)?;
    }
}

/// What a key press means, through the server's keyboard mapping.
fn key_event(keymap: &Keymap, keycode: u8, state: KeyButMask) -> Option<KeyEvent> {
    let ctrl = state.contains(KeyButMask::CONTROL);
    let key = match keymap.get(keycode, 0) {
        RETURN | KP_ENTER => Key::Enter,
        BACKSPACE => Key::Backspace,
        ESCAPE => Key::Escape,
        _ => {
            let mut shift = state.contains(KeyButMask::SHIFT);
            // With Num Lock on, keypad keys type their second level (the digits).
            if state.contains(KeyButMask::MOD2) && is_keypad(keymap.get(keycode, 1)) {
                shift = !shift;
            }
            let text = keymap.text(
                keycode,
                shift,
                state.contains(KeyButMask::LOCK),
                state.contains(KeyButMask::MOD5),
            );
            match text {
                Some(c) if !c.is_control() => Key::Text(c.to_string()),
                _ => Key::Other,
            }
        }
    };
    Some(KeyEvent { key, ctrl })
}

fn is_keypad(keysym: u32) -> bool {
    (0xff80..=0xffbd).contains(&keysym)
}

fn raise(conn: &RustConnection, window: xproto::Window) -> Result<()> {
    conn.configure_window(
        window,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    )?;
    Ok(())
}

fn invisible_cursor(conn: &RustConnection, root: xproto::Window) -> Result<xproto::Cursor> {
    let pixmap = conn.generate_id()?;
    conn.create_pixmap(1, pixmap, root, 1, 1)?;
    let cursor = conn.generate_id()?;
    conn.create_cursor(cursor, pixmap, pixmap, 0, 0, 0, 0, 0, 0, 0, 0)?;
    conn.free_pixmap(pixmap)?;
    Ok(cursor)
}

/// Grab the keyboard and pointer, retrying while someone else holds them.
fn grab(conn: &RustConnection, window: xproto::Window, cursor: xproto::Cursor) -> Result<()> {
    let mut keyboard = false;
    let mut pointer = false;
    for _ in 0..GRAB_ATTEMPTS {
        if !keyboard {
            let reply = conn
                .grab_keyboard(
                    false,
                    window,
                    CURRENT_TIME,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )?
                .reply()?;
            keyboard = reply.status == GrabStatus::SUCCESS;
        }
        if !pointer {
            let reply = conn
                .grab_pointer(
                    false,
                    window,
                    EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    NONE,
                    cursor,
                    CURRENT_TIME,
                )?
                .reply()?;
            pointer = reply.status == GrabStatus::SUCCESS;
        }
        if keyboard && pointer {
            return Ok(());
        }
        sleep(GRAB_RETRY);
    }
    let _ = conn.ungrab_keyboard(CURRENT_TIME);
    let _ = conn.ungrab_pointer(CURRENT_TIME);
    bail!(
        "could not grab the {} (another program holds it)",
        if keyboard { "pointer" } else { "keyboard" }
    );
}

/// The window's background pixmap and the monitors drawn into it.
struct Screen {
    window: xproto::Window,
    root: xproto::Window,
    depth: u8,
    has_randr: bool,
    /// MIT-SHM with fd passing works: the server reads frames straight from our memory.
    shm: bool,
    pixmap: xproto::Pixmap,
    gc: xproto::Gcontext,
    width: u16,
    height: u16,
    monitors: Vec<Monitor>,
}

/// (id, x, y, width, height)
type MonitorRect = (u32, i16, i16, u16, u16);

struct Monitor {
    output: Output,
    x: i16,
    y: i16,
    pixels: Pixels,
    dirty: bool,
    /// Never sent to the display yet: the whole of it must go, whatever the damage.
    fresh: bool,
}

/// A monitor's frame: in memory shared with the X server, or our own (sent over the
/// socket) when the server can't share.
enum Pixels {
    Heap(Vec<u8>),
    Shm {
        seg: shm::Seg,
        map: *mut u8,
        len: usize,
    },
}

impl Pixels {
    fn new(conn: &RustConnection, len: usize, use_shm: bool) -> Self {
        if use_shm {
            match Self::shared(conn, len) {
                Ok(pixels) => return pixels,
                Err(err) => eprintln!("warning: no shared memory with the X server: {err:#}"),
            }
        }
        Pixels::Heap(vec![0; len])
    }

    fn shared(conn: &RustConnection, len: usize) -> Result<Self> {
        // SAFETY: memfd_create with a valid name; the result is checked.
        let raw = unsafe { libc::memfd_create(c"tylock-frame".as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error()).context("memfd_create failed");
        }
        // SAFETY: a new descriptor nobody else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: plain syscalls on our own descriptor; results are checked.
        let map = unsafe {
            if libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) != 0 {
                return Err(std::io::Error::last_os_error()).context("ftruncate failed");
            }
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if map == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error()).context("mmap failed");
        }
        let seg = conn.generate_id()?;
        // Checked, so a server that can't map it is found out now, not mid-frame.
        if let Err(err) = conn
            .shm_attach_fd(seg, fd, true)
            .map_err(anyhow::Error::from)
            .and_then(|cookie| cookie.check().map_err(anyhow::Error::from))
        {
            // SAFETY: our own mapping, unused.
            unsafe { libc::munmap(map, len) };
            return Err(err).context("ShmAttachFd failed");
        }
        Ok(Pixels::Shm {
            seg,
            map: map.cast(),
            len,
        })
    }

    fn bytes(&mut self) -> &mut [u8] {
        match self {
            Pixels::Heap(data) => data,
            // SAFETY: a live read-write mapping of `len` bytes owned by this value; the
            // server only reads it during a frame we have waited for (see `run`).
            Pixels::Shm { map, len, .. } => unsafe { std::slice::from_raw_parts_mut(*map, *len) },
        }
    }

    /// Let go of the server's side (the mapping goes on drop).
    fn detach(&self, conn: &RustConnection) {
        if let Pixels::Shm { seg, .. } = self {
            let _ = conn.shm_detach(*seg);
        }
    }
}

impl Drop for Pixels {
    fn drop(&mut self) {
        if let Pixels::Shm { map, len, .. } = self {
            // SAFETY: unmapping our own mapping once.
            unsafe { libc::munmap(map.cast(), *len) };
        }
    }
}

impl Screen {
    fn new(
        conn: &RustConnection,
        window: xproto::Window,
        root: xproto::Window,
        depth: u8,
        has_randr: bool,
    ) -> Result<Self> {
        let shm = conn
            .extension_information(shm::X11_EXTENSION_NAME)?
            .is_some()
            && conn
                .shm_query_version()?
                .reply()
                .is_ok_and(|v| (v.major_version, v.minor_version) >= (1, 2));
        let mut screen = Self {
            window,
            root,
            depth,
            has_randr,
            shm,
            pixmap: NONE,
            gc: conn.generate_id()?,
            width: 0,
            height: 0,
            monitors: Vec::new(),
        };
        conn.create_gc(screen.gc, window, &CreateGCAux::new().graphics_exposures(0))?;
        screen.rebuild(conn)?;
        Ok(screen)
    }

    /// Follow the root window's size and the monitor layout.
    fn rebuild(&mut self, conn: &RustConnection) -> Result<()> {
        let geometry = conn.get_geometry(self.root)?.reply()?;
        let (width, height) = (geometry.width, geometry.height);
        if (width, height) != (self.width, self.height) {
            conn.configure_window(
                self.window,
                &ConfigureWindowAux::new()
                    .x(0)
                    .y(0)
                    .width(u32::from(width))
                    .height(u32::from(height)),
            )?;
            if self.pixmap != NONE {
                conn.free_pixmap(self.pixmap)?;
            }
            self.pixmap = conn.generate_id()?;
            conn.create_pixmap(self.depth, self.pixmap, self.window, width, height)?;
            (self.width, self.height) = (width, height);
        }
        let mut rects = self.query_monitors(conn).unwrap_or_default();
        if rects.is_empty() {
            rects.push((0, 0, 0, width, height));
        }
        for monitor in self.monitors.drain(..) {
            monitor.pixels.detach(conn);
        }
        for (id, x, y, w, h) in rects {
            // Clip to the screen; RandR can briefly report monitors outside it.
            let x0 = x.clamp(0, width as i16);
            let y0 = y.clamp(0, height as i16);
            let x1 = (i32::from(x) + i32::from(w)).min(i32::from(width));
            let y1 = (i32::from(y) + i32::from(h)).min(i32::from(height));
            let (w, h) = (x1 - i32::from(x0), y1 - i32::from(y0));
            if w <= 0 || h <= 0 {
                continue;
            }
            self.monitors.push(Monitor {
                output: Output {
                    id,
                    width: w as u32,
                    height: h as u32,
                },
                x: x0,
                y: y0,
                pixels: Pixels::new(conn, w as usize * h as usize * 4, self.shm),
                dirty: true,
                fresh: true,
            });
        }
        Ok(())
    }

    /// Active RandR monitors; the id is the monitor's name atom.
    fn query_monitors(&self, conn: &RustConnection) -> Result<Vec<MonitorRect>> {
        if !self.has_randr {
            return Ok(Vec::new());
        }
        let reply = conn.randr_get_monitors(self.root, true)?.reply()?;
        Ok(reply
            .monitors
            .iter()
            .map(|m| (m.name, m.x, m.y, m.width, m.height))
            .collect())
    }

    /// Draw the monitors that need it (all with `all`) and show them. Returns whether
    /// anything was sent. The caller must wait for the server to finish with a frame
    /// before drawing the next (the pixels may be shared with it).
    fn draw(
        &mut self,
        conn: &RustConnection,
        handler: &mut dyn Handler,
        all: bool,
    ) -> Result<bool> {
        let mut sent = Vec::new();
        for monitor in &mut self.monitors {
            if !(all || monitor.dirty) {
                continue;
            }
            let Output { width, height, .. } = monitor.output;
            let damage = handler.draw(
                &monitor.output,
                Canvas {
                    data: monitor.pixels.bytes(),
                    width,
                    height,
                    stride: width * 4,
                },
            );
            monitor.dirty = false;
            let full = Damage {
                x: 0,
                y: 0,
                width,
                height,
            };
            let area = match damage.filter(|_| !monitor.fresh) {
                Some(d) => match d.clip(width, height) {
                    Some(d) => d,
                    None => continue,
                },
                None => full,
            };
            monitor.fresh = false;
            let at = (monitor.x + area.x as i16, monitor.y + area.y as i16);
            match &mut monitor.pixels {
                Pixels::Shm { seg, .. } => {
                    conn.shm_put_image(
                        self.pixmap,
                        self.gc,
                        width as u16,
                        height as u16,
                        area.x as u16,
                        area.y as u16,
                        area.width as u16,
                        area.height as u16,
                        at.0,
                        at.1,
                        self.depth,
                        ImageFormat::Z_PIXMAP.into(),
                        false,
                        *seg,
                        0,
                    )?;
                }
                Pixels::Heap(data) => {
                    put_image(
                        conn,
                        self.pixmap,
                        self.gc,
                        self.depth,
                        at,
                        data,
                        width,
                        area,
                    )?;
                }
            }
            sent.push((at.0, at.1, area.width as u16, area.height as u16));
        }
        if sent.is_empty() {
            return Ok(false);
        }
        conn.change_window_attributes(
            self.window,
            &xproto::ChangeWindowAttributesAux::new().background_pixmap(self.pixmap),
        )?;
        // Repaint what changed from the background pixmap.
        for (x, y, width, height) in sent {
            conn.clear_area(false, self.window, x, y, width, height)?;
        }
        Ok(true)
    }

    fn free(&mut self, conn: &RustConnection) {
        for monitor in self.monitors.drain(..) {
            monitor.pixels.detach(conn);
        }
        let _ = conn.free_pixmap(self.pixmap);
        let _ = conn.free_gc(self.gc);
    }
}

/// `PutImage` of `area` of a frame `width` pixels wide, in strips that fit the server's
/// maximum request size.
#[allow(clippy::too_many_arguments)]
fn put_image(
    conn: &RustConnection,
    pixmap: xproto::Pixmap,
    gc: xproto::Gcontext,
    depth: u8,
    (x, y): (i16, i16),
    pixels: &[u8],
    width: u32,
    area: Damage,
) -> Result<()> {
    let stride = width as usize * 4;
    let row_bytes = area.width as usize * 4;
    let max = conn.maximum_request_bytes().saturating_sub(64);
    let rows_per = (max / row_bytes.max(1)).clamp(1, usize::from(u16::MAX));
    let mut strip = Vec::with_capacity(rows_per.min(area.height as usize) * row_bytes);
    let mut row = 0;
    while row < area.height as usize {
        let rows = rows_per.min(area.height as usize - row);
        strip.clear();
        for r in row..row + rows {
            let start = (area.y as usize + r) * stride + area.x as usize * 4;
            strip.extend_from_slice(&pixels[start..start + row_bytes]);
        }
        conn.put_image(
            ImageFormat::Z_PIXMAP,
            pixmap,
            gc,
            area.width as u16,
            rows as u16,
            x,
            y + row as i16,
            0,
            depth,
            &strip,
        )?;
        row += rows;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypad_range() {
        assert!(is_keypad(0xffb1)); // KP_1
        assert!(!is_keypad(0x31)); // 1
    }
}
