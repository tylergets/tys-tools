//! Keyboard input on X11, over x11rb: typing with XTest and global key listening with
//! XRecord. Keys are turned into characters with the server's keyboard mapping.

use std::collections::HashSet;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::record::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _, Mapping, ModMask};
use x11rb::protocol::xtest::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use xkbcommon::xkb;

use super::{InputKey, Key, KeyInput};

// Keysyms (X11/keysymdef.h).
const NO_SYMBOL: u32 = 0;
const SHIFT_L: u32 = 0xffe1;
const SHIFT_R: u32 = 0xffe2;
const CONTROL_L: u32 = 0xffe3;
const CONTROL_R: u32 = 0xffe4;
const CAPS_LOCK: u32 = 0xffe5;
const META_L: u32 = 0xffe7;
const META_R: u32 = 0xffe8;
const ALT_L: u32 = 0xffe9;
const ALT_R: u32 = 0xffea;
const SUPER_L: u32 = 0xffeb;
const SUPER_R: u32 = 0xffec;
const ISO_LEVEL3_SHIFT: u32 = 0xfe03;
const MODE_SWITCH: u32 = 0xff7e;
const ISO_LEFT_TAB: u32 = 0xfe20;
const RETURN: u32 = 0xff0d;
const TAB: u32 = 0xff09;
const ESCAPE: u32 = 0xff1b;
const BACKSPACE: u32 = 0xff08;
const SPACE: u32 = 0x0020;
const HOME: u32 = 0xff50;
const LEFT: u32 = 0xff51;
const UP: u32 = 0xff52;
const RIGHT: u32 = 0xff53;
const DOWN: u32 = 0xff54;
const PAGE_UP: u32 = 0xff55;
const PAGE_DOWN: u32 = 0xff56;
const END: u32 = 0xff57;
const DELETE: u32 = 0xffff;
const F1: u32 = 0xffbe;

impl Key {
    fn keysym(self) -> Result<u32> {
        Ok(match self {
            Key::Enter => RETURN,
            Key::Tab => TAB,
            Key::Escape => ESCAPE,
            Key::Backspace => BACKSPACE,
            Key::Space => SPACE,
            Key::Left => LEFT,
            Key::Right => RIGHT,
            Key::Up => UP,
            Key::Down => DOWN,
            Key::Home => HOME,
            Key::End => END,
            Key::Delete => DELETE,
            Key::PageUp => PAGE_UP,
            Key::PageDown => PAGE_DOWN,
            Key::F(n @ 1..=12) => F1 + u32::from(n) - 1,
            Key::F(n) => bail!("no such key: F{n}"),
        })
    }
}

/// The server's keycode → keysyms table (core protocol layout: group 1 levels 1–2, group 2
/// levels 1–2, then group 1 levels 3–4).
pub(crate) struct Keymap {
    min: u8,
    per: usize,
    syms: Vec<u32>,
}

impl Keymap {
    pub(crate) fn fetch(conn: &RustConnection) -> Result<Self> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let reply = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        Ok(Self {
            min,
            per: usize::from(reply.keysyms_per_keycode),
            syms: reply.keysyms,
        })
    }

    fn keycodes(&self) -> impl DoubleEndedIterator<Item = u8> + '_ {
        (0..self.syms.len() / self.per.max(1)).map(|i| self.min + i as u8)
    }

    pub(crate) fn get(&self, keycode: u8, index: usize) -> u32 {
        if index >= self.per || keycode < self.min {
            return NO_SYMBOL;
        }
        let at = usize::from(keycode - self.min) * self.per + index;
        self.syms.get(at).copied().unwrap_or(NO_SYMBOL)
    }

    fn set(&mut self, keycode: u8, keysym: u32) {
        let start = usize::from(keycode - self.min) * self.per;
        self.syms[start..start + self.per].fill(keysym);
    }

    /// The unshifted keysym, which names the physical key.
    fn base(&self, keycode: u8) -> u32 {
        self.get(keycode, 0)
    }

    /// A keycode that types `keysym` in group 1, and whether it needs Shift.
    fn find(&self, keysym: u32) -> Option<(u8, bool)> {
        [false, true].into_iter().find_map(|shift| {
            self.keycodes()
                .find(|&kc| self.get(kc, usize::from(shift)) == keysym)
                .map(|kc| (kc, shift))
        })
    }

    /// The character a press produces, following the core protocol's rules for Shift,
    /// Caps Lock and the level-3 shift (AltGr).
    pub(crate) fn text(&self, keycode: u8, shift: bool, caps: bool, level3: bool) -> Option<char> {
        let base = if level3 && self.get(keycode, 4) != NO_SYMBOL {
            4
        } else {
            0
        };
        let mut keysym = self.get(keycode, base + usize::from(shift));
        if keysym == NO_SYMBOL {
            keysym = self.get(keycode, base);
        }
        let c = char::from_u32(xkb::keysym_to_utf32(xkb::Keysym::new(keysym)))
            .filter(|&c| c != '\0')?;
        if caps && c.is_alphabetic() {
            let flipped = if shift {
                c.to_lowercase().next()
            } else {
                c.to_uppercase().next()
            };
            return flipped.or(Some(c));
        }
        // A shifted letter with no explicit level-2 keysym is its uppercase form.
        if shift && self.get(keycode, base + 1) == NO_SYMBOL {
            return c.to_uppercase().next();
        }
        Some(c)
    }
}

fn connect() -> Result<RustConnection> {
    let (conn, _) = x11rb::connect(None).context("failed to connect to the X server")?;
    Ok(conn)
}

fn require_extension(conn: &RustConnection, name: &'static str) -> Result<()> {
    if conn.extension_information(name)?.is_none() {
        bail!("the X server doesn't support the {name} extension");
    }
    Ok(())
}

/// Types into the focused window with XTest. Characters the keyboard layout doesn't have
/// are typed by briefly mapping them to an unused keycode.
pub(super) struct Typer {
    conn: RustConnection,
    keymap: Keymap,
    delay: Duration,
    shift: u8,
    spare: Option<u8>,
    spare_in_use: bool,
}

impl Typer {
    pub fn new(delay_ms: u32) -> Result<Self> {
        let conn = connect()?;
        require_extension(&conn, xtest::X11_EXTENSION_NAME)?;
        let keymap = Keymap::fetch(&conn)?;
        let shift = keymap
            .find(SHIFT_L)
            .or_else(|| keymap.find(SHIFT_R))
            .map(|(kc, _)| kc)
            .context("no Shift key in the keyboard mapping")?;
        let spare = keymap
            .keycodes()
            .rev()
            .find(|&kc| (0..keymap.per).all(|i| keymap.get(kc, i) == NO_SYMBOL));
        Ok(Self {
            conn,
            keymap,
            delay: Duration::from_millis(u64::from(delay_ms)),
            shift,
            spare,
            spare_in_use: false,
        })
    }

    pub fn type_text(&mut self, text: &str) -> Result<()> {
        let result = text.chars().try_for_each(|c| self.type_char(c));
        let restored = self.release_spare();
        result.and(restored)
    }

    pub fn tap(&mut self, key: Key, count: usize) -> Result<()> {
        let keysym = key.keysym()?;
        for _ in 0..count {
            self.type_keysym(keysym)?;
        }
        self.conn.sync()?;
        Ok(())
    }

    fn type_char(&mut self, c: char) -> Result<()> {
        let keysym = match c {
            '\n' | '\r' => RETURN,
            '\t' => TAB,
            _ => xkb::utf32_to_keysym(u32::from(c)).raw(),
        };
        if keysym == NO_SYMBOL {
            bail!("can't type {c:?}");
        }
        self.type_keysym(keysym)
    }

    fn type_keysym(&mut self, keysym: u32) -> Result<()> {
        let (keycode, shift) = match self.keymap.find(keysym) {
            Some(found) => found,
            None => (self.map_spare(keysym)?, false),
        };
        if shift {
            self.fake(xproto::KEY_PRESS_EVENT, self.shift)?;
        }
        self.fake(xproto::KEY_PRESS_EVENT, keycode)?;
        self.fake(xproto::KEY_RELEASE_EVENT, keycode)?;
        if shift {
            self.fake(xproto::KEY_RELEASE_EVENT, self.shift)?;
        }
        Ok(())
    }

    /// Point the spare keycode at `keysym` and return it.
    fn map_spare(&mut self, keysym: u32) -> Result<u8> {
        let spare = self
            .spare
            .context("no free keycode to type a character missing from the keyboard layout")?;
        self.remap(spare, keysym)?;
        self.spare_in_use = true;
        Ok(spare)
    }

    fn release_spare(&mut self) -> Result<()> {
        self.conn.sync()?;
        if let (Some(spare), true) = (self.spare, self.spare_in_use) {
            self.remap(spare, NO_SYMBOL)?;
            self.spare_in_use = false;
        }
        Ok(())
    }

    fn remap(&mut self, keycode: u8, keysym: u32) -> Result<()> {
        let per = self.keymap.per;
        self.conn
            .change_keyboard_mapping(1, keycode, per as u8, &vec![keysym; per])?
            .check()?;
        self.keymap.set(keycode, keysym);
        // Give clients a moment to pick up the new mapping.
        sleep(self.delay.max(Duration::from_millis(10)));
        Ok(())
    }

    fn fake(&self, kind: u8, keycode: u8) -> Result<()> {
        self.conn
            .xtest_fake_input(kind, keycode, x11rb::CURRENT_TIME, x11rb::NONE, 0, 0, 0)?;
        self.conn.flush()?;
        sleep(self.delay);
        Ok(())
    }
}

/// Call `on_key` for every key press and release in the X session, using XRecord.
pub fn listen(mut on_key: impl FnMut(KeyInput) + Send + 'static) -> Result<()> {
    // XRecord wants one connection for control and one that streams the recorded data.
    let ctrl = connect()?;
    let data = connect()?;
    require_extension(&ctrl, record::X11_EXTENSION_NAME)
        .context("failed to start global X11 key listener")?;

    let empty = record::Range8 { first: 0, last: 0 };
    let empty_ext = record::ExtRange {
        major: empty,
        minor: record::Range16 { first: 0, last: 0 },
    };
    let range = record::Range {
        core_requests: empty,
        core_replies: empty,
        ext_requests: empty_ext,
        ext_replies: empty_ext,
        // MappingNotify reaches every client; seeing it tells us the layout changed.
        delivered_events: record::Range8 {
            first: xproto::MAPPING_NOTIFY_EVENT,
            last: xproto::MAPPING_NOTIFY_EVENT,
        },
        device_events: record::Range8 {
            first: xproto::KEY_PRESS_EVENT,
            last: xproto::KEY_RELEASE_EVENT,
        },
        errors: empty,
        client_started: false,
        client_died: false,
    };
    let context = ctrl.generate_id()?;
    ctrl.record_create_context(context, 0, &[record::CS::ALL_CLIENTS.into()], &[range])?
        .check()
        .context("failed to start global X11 key listener")?;

    let mut keymap = Keymap::fetch(&ctrl)?;
    let root = ctrl.setup().roots[0].root;
    let mut caps = ctrl
        .query_pointer(root)?
        .reply()
        .is_ok_and(|reply| reply.mask.contains(ModMask::LOCK));
    let mut shifts = HashSet::new();
    let mut level3s = HashSet::new();

    const FROM_SERVER: u8 = 0;
    for reply in data.record_enable_context(context)? {
        let reply = reply?;
        if reply.category != FROM_SERVER || reply.client_swapped {
            continue;
        }
        // Each recorded event is a 32-byte core event.
        for event in reply.data.chunks_exact(32) {
            let keycode = event[1];
            let (pressed, keysym) = match event[0] & 0x7f {
                xproto::KEY_PRESS_EVENT => (true, keymap.base(keycode)),
                xproto::KEY_RELEASE_EVENT => (false, keymap.base(keycode)),
                xproto::MAPPING_NOTIFY_EVENT => {
                    if event[4] == u8::from(Mapping::KEYBOARD) {
                        keymap = Keymap::fetch(&ctrl)?;
                    }
                    continue;
                }
                _ => continue,
            };

            let held = match keysym {
                SHIFT_L | SHIFT_R => Some(&mut shifts),
                ISO_LEVEL3_SHIFT | MODE_SWITCH => Some(&mut level3s),
                _ => None,
            };
            if let Some(held) = held {
                if pressed {
                    held.insert(keycode);
                } else {
                    held.remove(&keycode);
                }
            }
            if keysym == CAPS_LOCK && pressed {
                caps = !caps;
            }

            let text = pressed
                .then(|| keymap.text(keycode, !shifts.is_empty(), caps, !level3s.is_empty()))
                .flatten();
            on_key(KeyInput {
                pressed,
                key: map_key(keysym),
                text,
            });
        }
    }
    Ok(())
}

fn map_key(keysym: u32) -> InputKey {
    match keysym {
        SHIFT_L | SHIFT_R => InputKey::Shift,
        CONTROL_L | CONTROL_R => InputKey::Ctrl,
        ALT_L | ALT_R | ISO_LEVEL3_SHIFT | MODE_SWITCH => InputKey::Alt,
        META_L | META_R | SUPER_L | SUPER_R => InputKey::Meta,
        CAPS_LOCK => InputKey::CapsLock,
        RETURN => InputKey::Enter,
        TAB | ISO_LEFT_TAB => InputKey::Tab,
        BACKSPACE => InputKey::Backspace,
        ESCAPE => InputKey::Escape,
        LEFT => InputKey::Left,
        RIGHT => InputKey::Right,
        UP => InputKey::Up,
        DOWN => InputKey::Down,
        HOME => InputKey::Home,
        END => InputKey::End,
        DELETE => InputKey::Delete,
        PAGE_UP => InputKey::PageUp,
        PAGE_DOWN => InputKey::PageDown,
        k if (F1..F1 + 12).contains(&k) => InputKey::F((k - F1 + 1) as u8),
        _ => InputKey::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// keycode 10: a/A, 11: 2/@ with level 3 ², 12: Shift_L, 13: q (no level 2).
    fn keymap() -> Keymap {
        let per = 6;
        let mut syms = vec![NO_SYMBOL; 4 * per];
        syms[..2].copy_from_slice(&['a' as u32, 'A' as u32]);
        syms[per..per + 2].copy_from_slice(&['2' as u32, '@' as u32]);
        syms[per + 4] = 0x00b2;
        syms[2 * per] = SHIFT_L;
        syms[3 * per] = 'q' as u32;
        Keymap { min: 10, per, syms }
    }

    #[test]
    fn finds_keycodes_for_keysyms() {
        let map = keymap();
        assert_eq!(map.find('a' as u32), Some((10, false)));
        assert_eq!(map.find('@' as u32), Some((11, true)));
        assert_eq!(map.find(SHIFT_L), Some((12, false)));
        assert_eq!(map.find('z' as u32), None);
    }

    #[test]
    fn translates_presses_to_text() {
        let map = keymap();
        assert_eq!(map.text(10, false, false, false), Some('a'));
        assert_eq!(map.text(10, true, false, false), Some('A'));
        assert_eq!(map.text(10, false, true, false), Some('A'));
        assert_eq!(map.text(10, true, true, false), Some('a'));
        assert_eq!(map.text(11, true, true, false), Some('@'));
        assert_eq!(map.text(11, false, false, true), Some('²'));
        assert_eq!(map.text(13, true, false, false), Some('Q'));
        assert_eq!(map.text(12, false, false, false), None);
    }

    #[test]
    fn maps_function_keys() {
        assert_eq!(map_key(F1), InputKey::F(1));
        assert_eq!(map_key(F1 + 11), InputKey::F(12));
        assert_eq!(map_key(RETURN), InputKey::Enter);
        assert_eq!(Key::F(12).keysym().unwrap(), F1 + 11);
    }
}
