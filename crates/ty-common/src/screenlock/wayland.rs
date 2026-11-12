//! The Wayland side of [`super::run`]: `ext-session-lock-v1`, one lock surface per output,
//! drawn into `wl_shm` buffers at the output's real resolution (fractional scale through
//! `wp_viewporter`, else the output's integer scale).

use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_compositor::WlCompositor,
    wl_keyboard::{self, WlKeyboard},
    wl_output::{self, WlOutput},
    wl_pointer::{self, WlPointer},
    wl_registry::{self, WlRegistry},
    wl_seat::{self, WlSeat},
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use xkbcommon::xkb;

use super::{Canvas, Handler, Key, KeyEvent, Output, WakeSource};

/// Buffers per surface: one on screen, one being drawn.
const MAX_BUFFERS: usize = 2;

pub fn run(handler: &mut dyn Handler, wake: WakeSource) -> Result<()> {
    let conn =
        Connection::connect_to_env().context("failed to connect to the Wayland compositor")?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut state = State::new();
    // Globals, then what they send on bind (output scales, seat capabilities, keymaps).
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;
    let Some(manager) = state.manager.clone() else {
        bail!("the compositor doesn't support ext-session-lock-v1, so it can't be locked");
    };
    if state.compositor.is_none() || state.shm.is_none() {
        bail!("the compositor lacks wl_compositor or wl_shm");
    }

    state.lock = Some(manager.lock(&qh, ()));
    for index in 0..state.outputs.len() {
        state.add_lock_surface(index, &qh);
    }

    let mut announced = false;
    let mut unlock_requested = false;
    if let Some(on) = state.caps_changed.take() {
        handler.caps_lock(on);
    }
    loop {
        queue.dispatch_pending(&mut state)?;
        if state.finished {
            if !state.locked {
                bail!("the compositor refused to lock the screen (is another locker running?)");
            }
            // The compositor ended the lock itself; there is nothing left to unlock.
            return Ok(());
        }
        if let Some(on) = state.caps_changed.take() {
            handler.caps_lock(on);
        }
        state.keyboard.push_repeats(&mut state.keys);
        for key in state.keys.drain(..) {
            handler.key(key);
        }

        let control = handler.control();
        unlock_requested |= control.unlock;
        // Unlocking before `locked` is a protocol error; wait for it.
        if unlock_requested && state.locked {
            if let Some(lock) = state.lock.take() {
                lock.unlock_and_destroy();
            }
            state.outputs.clear();
            queue.roundtrip(&mut state)?;
            return Ok(());
        }
        state.draw(handler, control.redraw, &qh)?;
        if state.locked && !announced {
            announced = true;
            handler.locked();
        }

        queue.flush()?;
        let deadline = super::earliest(control.wake_at, state.keyboard.next_repeat());
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd().as_raw_fd();
            let ready = super::wait(&[fd], &wake, deadline)?;
            if ready[0] {
                match guard.read() {
                    Ok(_) => {}
                    Err(WaylandError::Io(err)) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(err) => return Err(err).context("lost the Wayland connection"),
                }
            }
        }
    }
}

struct State {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    manager: Option<ExtSessionLockManagerV1>,
    viewporter: Option<WpViewporter>,
    fractional: Option<WpFractionalScaleManagerV1>,
    outputs: Vec<OutputState>,
    keyboards: Vec<WlKeyboard>,
    pointers: Vec<WlPointer>,
    lock: Option<ExtSessionLockV1>,
    locked: bool,
    finished: bool,
    keyboard: Keyboard,
    keys: Vec<KeyEvent>,
    caps_changed: Option<bool>,
}

struct OutputState {
    /// The output's registry name, used as the [`Output::id`].
    name: u32,
    output: WlOutput,
    /// `wl_output.scale`, used when there is no fractional scale.
    scale: i32,
    surface: Option<LockSurface>,
}

struct LockSurface {
    surface: WlSurface,
    lock_surface: ExtSessionLockSurfaceV1,
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
    /// Logical size from the last configure.
    size: Option<(u32, u32)>,
    /// Preferred fractional scale, in 120ths.
    fraction: Option<u32>,
    buffers: Vec<Buffer>,
    /// Needs drawing: just configured, rescaled, or a redraw didn't find a free buffer.
    dirty: bool,
}

impl Drop for LockSurface {
    fn drop(&mut self) {
        self.buffers.clear();
        if let Some(viewport) = &self.viewport {
            viewport.destroy();
        }
        if let Some(fractional) = &self.fractional {
            fractional.destroy();
        }
        self.lock_surface.destroy();
        self.surface.destroy();
    }
}

impl LockSurface {
    /// The buffer size in physical pixels, and the integer buffer scale to set (1 when the
    /// viewport scales instead).
    fn buffer_size(&self, output_scale: i32) -> Option<(u32, u32, i32)> {
        let (width, height) = self.size?;
        if let (Some(fraction), Some(_)) = (self.fraction, &self.viewport) {
            // Rounded half away from zero, as the protocol asks.
            let scale = |v: u32| (v * fraction + 60) / 120;
            return Some((scale(width), scale(height), 1));
        }
        let scale = output_scale.max(1);
        Some((width * scale as u32, height * scale as u32, scale))
    }
}

impl State {
    fn new() -> Self {
        Self {
            compositor: None,
            shm: None,
            manager: None,
            viewporter: None,
            fractional: None,
            outputs: Vec::new(),
            keyboards: Vec::new(),
            pointers: Vec::new(),
            lock: None,
            locked: false,
            finished: false,
            keyboard: Keyboard::new(),
            keys: Vec::new(),
            caps_changed: None,
        }
    }

    fn add_lock_surface(&mut self, index: usize, qh: &QueueHandle<Self>) {
        let (Some(lock), Some(compositor)) = (&self.lock, &self.compositor) else {
            return;
        };
        let output = &mut self.outputs[index];
        if output.surface.is_some() {
            return;
        }
        let name = output.name;
        let surface = compositor.create_surface(qh, ());
        let viewport = self
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, qh, ()));
        let fractional = self
            .fractional
            .as_ref()
            .filter(|_| viewport.is_some())
            .map(|manager| manager.get_fractional_scale(&surface, qh, name));
        let lock_surface = lock.get_lock_surface(&surface, &output.output, qh, name);
        output.surface = Some(LockSurface {
            surface,
            lock_surface,
            viewport,
            fractional,
            size: None,
            fraction: None,
            buffers: Vec::new(),
            dirty: false,
        });
    }

    fn surface_mut(&mut self, name: u32) -> Option<&mut LockSurface> {
        self.outputs
            .iter_mut()
            .find(|output| output.name == name)
            .and_then(|output| output.surface.as_mut())
    }

    /// Draw every surface that needs it (all of them with `all`).
    fn draw(&mut self, handler: &mut dyn Handler, all: bool, qh: &QueueHandle<Self>) -> Result<()> {
        let Some(shm) = self.shm.clone() else {
            return Ok(());
        };
        for output in &mut self.outputs {
            let output_scale = output.scale;
            let Some(lock) = output.surface.as_mut() else {
                continue;
            };
            if all {
                lock.dirty = true;
            }
            if !lock.dirty {
                continue;
            }
            let Some((width, height, buffer_scale)) = lock.buffer_size(output_scale) else {
                continue;
            };
            if width == 0 || height == 0 {
                continue;
            }
            // Free buffers of an old size go; busy ones are kept until released.
            lock.buffers
                .retain(|b| b.busy() || (b.width == width && b.height == height));
            let index = match lock
                .buffers
                .iter()
                .position(|b| !b.busy() && b.width == width && b.height == height)
            {
                Some(index) => index,
                None if lock.buffers.len() < MAX_BUFFERS => {
                    lock.buffers.push(Buffer::new(&shm, qh, width, height)?);
                    lock.buffers.len() - 1
                }
                // Every buffer is on screen: draw when one is released.
                None => continue,
            };
            let buffer = &mut lock.buffers[index];
            let damage = handler.draw(
                &Output {
                    id: output.name,
                    width,
                    height,
                },
                Canvas {
                    data: buffer.pixels(),
                    width,
                    height,
                    stride: width * 4,
                },
            );
            let (logical_w, logical_h) = lock.size.unwrap_or((width, height));
            lock.surface.attach(Some(&buffer.buffer), 0, 0);
            match &lock.viewport {
                Some(viewport) if buffer_scale == 1 && lock.fraction.is_some() => {
                    lock.surface.set_buffer_scale(1);
                    viewport.set_destination(logical_w as i32, logical_h as i32);
                }
                _ => lock.surface.set_buffer_scale(buffer_scale),
            }
            // Damage is a hint only: the buffer had a full frame drawn into it anyway.
            match damage.and_then(|d| d.clip(width, height)) {
                Some(d) => lock.surface.damage_buffer(
                    d.x as i32,
                    d.y as i32,
                    d.width as i32,
                    d.height as i32,
                ),
                None => lock.surface.damage_buffer(0, 0, i32::MAX, i32::MAX),
            }
            lock.surface.commit();
            buffer.set_busy();
            lock.dirty = false;
        }
        Ok(())
    }
}

/// One `wl_shm` buffer in its own memfd.
struct Buffer {
    buffer: WlBuffer,
    busy: Arc<AtomicBool>,
    map: *mut u8,
    len: usize,
    width: u32,
    height: u32,
    _fd: OwnedFd,
}

impl Buffer {
    fn new(shm: &WlShm, qh: &QueueHandle<State>, width: u32, height: u32) -> Result<Self> {
        let stride = width as usize * 4;
        let len = stride * height as usize;
        // SAFETY: memfd_create with a valid name; the result is checked.
        let raw = unsafe { libc::memfd_create(c"tylock-buffer".as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error()).context("memfd_create failed");
        }
        // SAFETY: `raw` is a new descriptor nobody else owns.
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
        let busy = Arc::new(AtomicBool::new(false));
        let pool = shm.create_pool(fd.as_fd(), len as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Xrgb8888,
            qh,
            busy.clone(),
        );
        pool.destroy();
        Ok(Self {
            buffer,
            busy,
            map: map.cast(),
            len,
            width,
            height,
            _fd: fd,
        })
    }

    fn pixels(&mut self) -> &mut [u8] {
        // SAFETY: `map` is a live read-write mapping of `len` bytes, owned by this buffer;
        // the compositor doesn't read it while the buffer isn't busy.
        unsafe { std::slice::from_raw_parts_mut(self.map, self.len) }
    }

    fn busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed)
    }

    fn set_busy(&self) {
        self.busy.store(true, Ordering::Relaxed);
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        // SAFETY: unmapping our own mapping once; nothing borrows it past this point.
        unsafe {
            libc::munmap(self.map.cast(), self.len);
        }
    }
}

/// Keymap, modifier state and key repeat for the seat's keyboard.
struct Keyboard {
    context: xkb::Context,
    keymap: Option<xkb::Keymap>,
    state: Option<xkb::State>,
    /// Repeats per second (0 turns repeat off) and the delay before the first.
    rate: i32,
    delay: i32,
    /// The held key and when it repeats next.
    repeat: Option<(xkb::Keycode, Instant)>,
}

impl Keyboard {
    fn new() -> Self {
        Self {
            context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            keymap: None,
            state: None,
            rate: 25,
            delay: 600,
            repeat: None,
        }
    }

    fn event(&self, keycode: xkb::Keycode) -> Option<KeyEvent> {
        let state = self.state.as_ref()?;
        let sym = state.key_get_one_sym(keycode);
        let ctrl = state.mod_name_is_active(xkb::MOD_NAME_CTRL, xkb::STATE_MODS_EFFECTIVE);
        let key = match sym.raw() {
            xkb::keysyms::KEY_Return | xkb::keysyms::KEY_KP_Enter => Key::Enter,
            xkb::keysyms::KEY_BackSpace => Key::Backspace,
            xkb::keysyms::KEY_Escape => Key::Escape,
            _ => {
                // With Ctrl held, xkb gives control characters; the keysym's own character
                // is what Ctrl+U and friends need.
                let text = if ctrl {
                    char::from_u32(xkb::keysym_to_utf32(sym))
                        .filter(|&c| c != '\0')
                        .map(String::from)
                        .unwrap_or_default()
                } else {
                    state.key_get_utf8(keycode)
                };
                if text.is_empty() || text.chars().any(char::is_control) {
                    Key::Other
                } else {
                    Key::Text(text)
                }
            }
        };
        Some(KeyEvent { key, ctrl })
    }

    fn press(&mut self, keycode: xkb::Keycode, keys: &mut Vec<KeyEvent>) {
        if let Some(event) = self.event(keycode) {
            keys.push(event);
        }
        let repeats = self
            .keymap
            .as_ref()
            .is_some_and(|keymap| keymap.key_repeats(keycode));
        self.repeat = (repeats && self.rate > 0).then(|| {
            (
                keycode,
                Instant::now() + Duration::from_millis(self.delay.max(0) as u64),
            )
        });
    }

    fn release(&mut self, keycode: xkb::Keycode) {
        if self.repeat.is_some_and(|(held, _)| held == keycode) {
            self.repeat = None;
        }
    }

    fn next_repeat(&self) -> Option<Instant> {
        self.repeat.map(|(_, at)| at)
    }

    /// Key events for repeats that are due.
    fn push_repeats(&mut self, keys: &mut Vec<KeyEvent>) {
        let Some((keycode, mut at)) = self.repeat else {
            return;
        };
        let interval = Duration::from_secs(1) / self.rate.max(1) as u32;
        let now = Instant::now();
        while at <= now {
            if let Some(event) = self.event(keycode) {
                keys.push(event);
            }
            at += interval;
        }
        self.repeat = Some((keycode, at));
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "ext_session_lock_manager_v1" => {
                    state.manager = Some(registry.bind(name, 1, qh, ()));
                }
                "wp_viewporter" => state.viewporter = Some(registry.bind(name, 1, qh, ())),
                "wp_fractional_scale_manager_v1" => {
                    state.fractional = Some(registry.bind(name, 1, qh, ()));
                }
                "wl_seat" => {
                    let _: WlSeat = registry.bind(name, version.min(5), qh, ());
                }
                "wl_output" => {
                    let output = registry.bind(name, version.min(3), qh, name);
                    state.outputs.push(OutputState {
                        name,
                        output,
                        scale: 1,
                        surface: None,
                    });
                    // An output plugged in while locked gets a lock surface too.
                    let index = state.outputs.len() - 1;
                    state.add_lock_surface(index, qh);
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = state.outputs.iter().position(|o| o.name == name) {
                    let output = state.outputs.remove(index);
                    drop(output.surface);
                    output.output.release();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Scale { factor } = event {
            if let Some(output) = state.outputs.iter_mut().find(|o| o.name == *name) {
                if output.scale != factor {
                    output.scale = factor;
                    if let Some(surface) = output.surface.as_mut() {
                        surface.dirty = true;
                    }
                }
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        else {
            return;
        };
        // One keyboard and pointer per seat is plenty; capabilities rarely change.
        if caps.contains(wl_seat::Capability::Keyboard) && state.keyboards.is_empty() {
            state.keyboards.push(seat.get_keyboard(qh, ()));
        }
        if caps.contains(wl_seat::Capability::Pointer) && state.pointers.is_empty() {
            state.pointers.push(seat.get_pointer(qh, ()));
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let keyboard = &mut state.keyboard;
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                if format != WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    return;
                }
                // SAFETY: the compositor sent a keymap fd of `size` bytes in text format.
                let keymap = unsafe {
                    xkb::Keymap::new_from_fd(
                        &keyboard.context,
                        fd,
                        size as usize,
                        xkb::KEYMAP_FORMAT_TEXT_V1,
                        xkb::KEYMAP_COMPILE_NO_FLAGS,
                    )
                };
                match keymap {
                    Ok(Some(keymap)) => {
                        keyboard.state = Some(xkb::State::new(&keymap));
                        keyboard.keymap = Some(keymap);
                    }
                    _ => eprintln!("warning: could not load the compositor's keymap"),
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } => {
                let keycode = xkb::Keycode::new(key + 8);
                match key_state {
                    WEnum::Value(wl_keyboard::KeyState::Pressed) => {
                        keyboard.press(keycode, &mut state.keys);
                    }
                    _ => keyboard.release(keycode),
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(xkb_state) = keyboard.state.as_mut() {
                    xkb_state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                    let caps =
                        xkb_state.mod_name_is_active(xkb::MOD_NAME_CAPS, xkb::STATE_MODS_EFFECTIVE);
                    state.caps_changed = Some(caps);
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                keyboard.rate = rate;
                keyboard.delay = delay;
            }
            wl_keyboard::Event::Leave { .. } => keyboard.repeat = None,
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        _: &mut Self,
        pointer: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // No cursor on the lock screen.
        if let wl_pointer::Event::Enter { serial, .. } = event {
            pointer.set_cursor(serial, None, 0, 0);
        }
    }
}

impl Dispatch<ExtSessionLockV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => state.locked = true,
            ext_session_lock_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, u32> for State {
    fn event(
        state: &mut Self,
        lock_surface: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            lock_surface.ack_configure(serial);
            if let Some(surface) = state.surface_mut(*name) {
                surface.size = Some((width, height));
                surface.dirty = true;
            }
        }
    }
}

impl Dispatch<WpFractionalScaleV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            if let Some(surface) = state.surface_mut(*name) {
                if surface.fraction != Some(scale) {
                    surface.fraction = Some(scale);
                    surface.dirty = true;
                }
            }
        }
    }
}

impl Dispatch<WlBuffer, Arc<AtomicBool>> for State {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        event: wl_buffer::Event,
        busy: &Arc<AtomicBool>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            busy.store(false, Ordering::Relaxed);
        }
    }
}

delegate_noop!(State: WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ExtSessionLockManagerV1);
delegate_noop!(State: WpViewporter);
delegate_noop!(State: WpViewport);
delegate_noop!(State: WpFractionalScaleManagerV1);
