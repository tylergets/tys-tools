//! Reading keyboards and mice straight from `/dev/input`, for Wayland, where clients can't
//! see each other's input. Devices plugged in later are picked up by a periodic rescan.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use evdev::{Device, EventSummary, KeyCode};
use xkbcommon::xkb;

use super::{single_char, InputKey, KeyInput};
use crate::wm::buttons;

const INPUT_DIR: &str = "/dev/input";
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);
/// Key codes from here on are mouse/joystick buttons, not keys.
const FIRST_BUTTON: u16 = 0x100;

/// A key event from a device: `(code, value)` with value 0 = release, 1 = press, 2 = repeat.
type RawKey = (u16, i32);

/// Read key events from every device that `filter` accepts, now and as they appear, until
/// `stop` is set. Fails if no such device can be opened.
fn watch_devices(
    filter: fn(&Device) -> bool,
    what: &str,
    tx: Sender<RawKey>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let opened = Arc::new(Mutex::new(HashSet::new()));
    let denied = scan(filter, &tx, &opened, &stop)?;
    if opened.lock().unwrap().is_empty() {
        if denied {
            bail!(
                "no permission to read {what} in {INPUT_DIR}; on Wayland this needs read \
access to the input devices (e.g. add your user to the `input` group and log in again)"
            );
        }
        bail!("no {what} found in {INPUT_DIR}");
    }
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(RESCAN_INTERVAL);
            if scan(filter, &tx, &opened, &stop).is_err() {
                return;
            }
        }
    });
    Ok(())
}

/// Open devices not already open. Returns whether any device was unreadable.
fn scan(
    filter: fn(&Device) -> bool,
    tx: &Sender<RawKey>,
    opened: &Arc<Mutex<HashSet<PathBuf>>>,
    stop: &Arc<AtomicBool>,
) -> Result<bool> {
    let mut denied = false;
    let entries =
        std::fs::read_dir(INPUT_DIR).with_context(|| format!("cannot list {INPUT_DIR}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_event_node = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("event"));
        if !is_event_node || opened.lock().unwrap().contains(&path) {
            continue;
        }
        match Device::open(&path) {
            Ok(device) if filter(&device) => {
                opened.lock().unwrap().insert(path.clone());
                spawn_reader(path, device, tx.clone(), opened.clone(), stop.clone());
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => denied = true,
            Err(_) => {}
        }
    }
    Ok(denied)
}

fn spawn_reader(
    path: PathBuf,
    mut device: Device,
    tx: Sender<RawKey>,
    opened: Arc<Mutex<HashSet<PathBuf>>>,
    stop: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        loop {
            let Ok(events) = device.fetch_events() else {
                // Unplugged: let a later scan reopen it if it comes back.
                break;
            };
            for event in events {
                if let EventSummary::Key(_, code, value) = event.destructure() {
                    if stop.load(Ordering::Relaxed) || tx.send((code.0, value)).is_err() {
                        return;
                    }
                }
            }
        }
        forget(&opened, &path);
    });
}

fn forget(opened: &Mutex<HashSet<PathBuf>>, path: &Path) {
    opened.lock().unwrap().remove(path);
}

fn is_keyboard(device: &Device) -> bool {
    device
        .supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::KEY_A) && keys.contains(KeyCode::KEY_ENTER))
}

fn is_pointer(device: &Device) -> bool {
    device
        .supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::BTN_LEFT))
}

/// Report every key press and release on every keyboard. Blocks.
pub fn listen(mut on_key: impl FnMut(KeyInput) + Send + 'static) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    watch_devices(
        is_keyboard,
        "keyboards",
        tx,
        Arc::new(AtomicBool::new(false)),
    )?;

    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    // Empty names use the XKB_DEFAULT_* environment variables, then the system default.
    let keymap =
        xkb::Keymap::new_from_names(&context, "", "", "", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
            .context("failed to load the keyboard layout (check XKB_DEFAULT_LAYOUT)")?;
    let mut state = xkb::State::new(&keymap);

    for (code, value) in rx {
        if code >= FIRST_BUTTON {
            continue;
        }
        // evdev codes are offset by 8 in XKB.
        let keycode = xkb::Keycode::new(u32::from(code) + 8);
        let pressed = value != 0;
        let text = pressed
            .then(|| single_char(&state.key_get_utf8(keycode)))
            .flatten();
        if value != 2 {
            let direction = if pressed {
                xkb::KeyDirection::Down
            } else {
                xkb::KeyDirection::Up
            };
            state.update_key(keycode, direction);
        }
        on_key(KeyInput {
            pressed,
            key: map_key(KeyCode(code)),
            text,
        });
    }
    bail!("stopped receiving key events")
}

fn map_key(code: KeyCode) -> InputKey {
    match code {
        KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT => InputKey::Shift,
        KeyCode::KEY_LEFTCTRL | KeyCode::KEY_RIGHTCTRL => InputKey::Ctrl,
        KeyCode::KEY_LEFTALT | KeyCode::KEY_RIGHTALT => InputKey::Alt,
        KeyCode::KEY_LEFTMETA | KeyCode::KEY_RIGHTMETA => InputKey::Meta,
        KeyCode::KEY_CAPSLOCK => InputKey::CapsLock,
        KeyCode::KEY_ENTER | KeyCode::KEY_KPENTER => InputKey::Enter,
        KeyCode::KEY_TAB => InputKey::Tab,
        KeyCode::KEY_BACKSPACE => InputKey::Backspace,
        KeyCode::KEY_ESC => InputKey::Escape,
        KeyCode::KEY_LEFT => InputKey::Left,
        KeyCode::KEY_RIGHT => InputKey::Right,
        KeyCode::KEY_UP => InputKey::Up,
        KeyCode::KEY_DOWN => InputKey::Down,
        KeyCode::KEY_HOME => InputKey::Home,
        KeyCode::KEY_END => InputKey::End,
        KeyCode::KEY_DELETE => InputKey::Delete,
        KeyCode::KEY_PAGEUP => InputKey::PageUp,
        KeyCode::KEY_PAGEDOWN => InputKey::PageDown,
        KeyCode::KEY_F1 => InputKey::F(1),
        KeyCode::KEY_F2 => InputKey::F(2),
        KeyCode::KEY_F3 => InputKey::F(3),
        KeyCode::KEY_F4 => InputKey::F(4),
        KeyCode::KEY_F5 => InputKey::F(5),
        KeyCode::KEY_F6 => InputKey::F(6),
        KeyCode::KEY_F7 => InputKey::F(7),
        KeyCode::KEY_F8 => InputKey::F(8),
        KeyCode::KEY_F9 => InputKey::F(9),
        KeyCode::KEY_F10 => InputKey::F(10),
        KeyCode::KEY_F11 => InputKey::F(11),
        KeyCode::KEY_F12 => InputKey::F(12),
        _ => InputKey::Other,
    }
}

/// Which mouse buttons are held, read from every pointing device. Stops when dropped.
pub struct PointerButtons {
    state: Arc<AtomicU8>,
    stop: Arc<AtomicBool>,
}

impl PointerButtons {
    pub fn watch() -> Result<Self> {
        let (tx, rx) = mpsc::channel::<RawKey>();
        let stop = Arc::new(AtomicBool::new(false));
        watch_devices(is_pointer, "pointing devices", tx, stop.clone())?;
        let state = Arc::new(AtomicU8::new(0));
        {
            let state = state.clone();
            thread::spawn(move || {
                for (code, value) in rx {
                    let bit = match KeyCode(code) {
                        KeyCode::BTN_LEFT => buttons::LEFT,
                        KeyCode::BTN_MIDDLE => buttons::MIDDLE,
                        KeyCode::BTN_RIGHT => buttons::RIGHT,
                        _ => continue,
                    };
                    if value == 0 {
                        state.fetch_and(!bit, Ordering::Relaxed);
                    } else {
                        state.fetch_or(bit, Ordering::Relaxed);
                    }
                }
            });
        }
        Ok(Self { state, stop })
    }

    /// Bitmask of [`buttons`] currently held.
    pub fn state(&self) -> u8 {
        self.state.load(Ordering::Relaxed)
    }
}

impl Drop for PointerButtons {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_keys() {
        assert_eq!(map_key(KeyCode::KEY_LEFTSHIFT), InputKey::Shift);
        assert_eq!(map_key(KeyCode::KEY_F7), InputKey::F(7));
        assert_eq!(map_key(KeyCode::KEY_A), InputKey::Other);
    }

    #[test]
    fn translates_with_default_layout() {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let Some(keymap) = xkb::Keymap::new_from_names(&context, "", "", "us", "", None, 0) else {
            // No XKB data in this environment.
            return;
        };
        let mut state = xkb::State::new(&keymap);
        let a = xkb::Keycode::new(u32::from(KeyCode::KEY_A.0) + 8);
        let shift = xkb::Keycode::new(u32::from(KeyCode::KEY_LEFTSHIFT.0) + 8);
        assert_eq!(state.key_get_utf8(a), "a");
        state.update_key(shift, xkb::KeyDirection::Down);
        assert_eq!(state.key_get_utf8(a), "A");
    }
}
