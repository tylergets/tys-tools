//! Passive X11 key grabs. Wayland users bind the corresponding CLI in their compositor.
use anyhow::{bail, ensure, Context, Result};
use std::sync::mpsc::{self, Receiver};
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{ConnectionExt, GrabMode, ModMask},
        Event,
    },
    rust_connection::RustConnection,
};
use xkbcommon::xkb;

fn parse(chord: &str) -> Result<(u16, u32)> {
    ensure!(
        !chord.contains('\0'),
        "shortcut cannot contain a NUL character"
    );
    let mut parts = chord.split('+').peekable();
    let mut mask = 0;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            let sym = xkb::keysym_from_name(part, xkb::KEYSYM_CASE_INSENSITIVE).raw();
            ensure!(sym != 0, "unknown shortcut key: {part}");
            return Ok((mask, sym));
        }
        mask |= match part.to_ascii_lowercase().as_str() {
            "shift" => 1,
            "ctrl" | "control" => 4,
            "alt" | "mod1" => 8,
            "mod2" => 16,
            "mod3" => 32,
            "super" | "meta" | "mod4" => 64,
            "mod5" => 128,
            _ => bail!("unknown shortcut modifier: {part}"),
        };
    }
    bail!("empty shortcut")
}

pub fn validate_shortcut(chord: &str) -> Result<()> {
    parse(chord).map(|_| ())
}

struct Binding {
    keycode: u8,
    modifiers: u16,
    ignored: u16,
    index: usize,
}

fn grab(conn: &RustConnection, root: u32, chords: &[(u16, u32)]) -> Result<Vec<Binding>> {
    let setup = conn.setup();
    let map = conn
        .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)?
        .reply()?;
    let per = usize::from(map.keysyms_per_keycode);
    ensure!(per > 0, "empty keyboard mapping");
    let modifiers = conn.get_modifier_mapping()?.reply()?;
    let per_modifier = modifiers.keycodes.len() / 8;
    let mut locks = 2u16; // Caps Lock
    if per_modifier > 0 {
        for (i, codes) in modifiers.keycodes.chunks(per_modifier).enumerate() {
            if codes.iter().any(|code| {
                code.checked_sub(setup.min_keycode)
                    .and_then(|n| {
                        map.keysyms
                            .get(usize::from(n) * per..usize::from(n) * per + per)
                    })
                    .is_some_and(|syms| syms.contains(&0xff7f)) // Num Lock
            }) {
                locks |= 1 << i;
            }
        }
    }
    let mut bindings: Vec<Binding> = Vec::new();
    for (index, &(requested, sym)) in chords.iter().enumerate() {
        let mut found = false;
        for (offset, syms) in map.keysyms.chunks(per).enumerate() {
            // Core keyboard group 1; a shifted keysym implies Shift.
            let Some(level) = syms.iter().take(2).position(|s| *s == sym) else {
                continue;
            };
            found = true;
            let modifiers = requested | if level == 1 { 1 } else { 0 };
            let ignored = locks & !modifiers;
            let keycode = setup.min_keycode + offset as u8;
            ensure!(
                !bindings.iter().any(|b| b.keycode == keycode
                    && (b.modifiers ^ modifiers) & !(b.ignored | ignored) == 0),
                "duplicate shortcut binding"
            );
            // Every subset of the lock modifiers gets a passive grab.
            for extra in 0u16..256 {
                if extra & !ignored != 0 {
                    continue;
                }
                conn.grab_key(
                    false,
                    root,
                    ModMask::from(modifiers | extra),
                    keycode,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )?
                .check()
                .context("cannot grab shortcut (already bound by another application?)")?;
            }
            bindings.push(Binding {
                keycode,
                modifiers,
                ignored,
                index,
            });
        }
        ensure!(
            found,
            "shortcut keysym {sym:#x} is not in the current keyboard layout"
        );
    }
    conn.flush()?;
    Ok(bindings)
}

/// Register all chords before returning. Each event is the index into `chords`.
/// The listener ends when the receiving side is dropped and another key arrives.
pub fn global_shortcuts(chords: &[String]) -> Result<Receiver<usize>> {
    ensure!(
        !crate::session::is_wayland(),
        "global shortcuts require X11; bind the CLI in your Wayland compositor"
    );
    let chords = chords
        .iter()
        .map(|s| parse(s))
        .collect::<Result<Vec<_>>>()?;
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let mut bindings = grab(&conn, root, &chords)?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut listen = || -> Result<()> {
            loop {
                match conn.wait_for_event()? {
                    Event::KeyPress(event) => {
                        let modifiers = u16::from(event.state) & 0xff;
                        if let Some(binding) = bindings.iter().find(|b| {
                            b.keycode == event.detail && modifiers & !b.ignored == b.modifiers
                        }) {
                            if tx.send(binding.index).is_err() {
                                return Ok(());
                            }
                        }
                    }
                    Event::MappingNotify(_) => {
                        conn.ungrab_key(0u8, root, ModMask::ANY)?.check()?;
                        bindings = grab(&conn, root, &chords)?;
                    }
                    _ => {}
                }
            }
        };
        if let Err(error) = listen() {
            eprintln!("Global shortcuts: {error:#}");
        }
    });
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_chords_and_rejects_invalid_keys() {
        assert_eq!(parse("ctrl+mod4+space").unwrap(), (68, 32));
        assert!(parse("ctrl+").is_err());
        assert!(parse("unknown+space").is_err());
    }
}
