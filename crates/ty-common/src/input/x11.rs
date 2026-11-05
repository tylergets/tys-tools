//! Global key listening on X11 (XRecord, via rdev).

use anyhow::{anyhow, Result};
use rdev::{EventType, Key};

use super::{single_char, InputKey, KeyInput};

pub fn listen(mut on_key: impl FnMut(KeyInput) + Send + 'static) -> Result<()> {
    rdev::listen(move |event| {
        let input = match event.event_type {
            EventType::KeyPress(key) => KeyInput {
                pressed: true,
                key: map_key(key),
                text: event.name.as_deref().and_then(single_char),
            },
            EventType::KeyRelease(key) => KeyInput {
                pressed: false,
                key: map_key(key),
                text: None,
            },
            _ => return,
        };
        on_key(input);
    })
    .map_err(|err| anyhow!("failed to start global X11 key listener: {err:?}"))
}

fn map_key(key: Key) -> InputKey {
    match key {
        Key::ShiftLeft | Key::ShiftRight => InputKey::Shift,
        Key::ControlLeft | Key::ControlRight => InputKey::Ctrl,
        Key::Alt | Key::AltGr => InputKey::Alt,
        Key::MetaLeft | Key::MetaRight => InputKey::Meta,
        Key::CapsLock => InputKey::CapsLock,
        Key::Return => InputKey::Enter,
        Key::Tab => InputKey::Tab,
        Key::Backspace => InputKey::Backspace,
        Key::Escape => InputKey::Escape,
        Key::LeftArrow => InputKey::Left,
        Key::RightArrow => InputKey::Right,
        Key::UpArrow => InputKey::Up,
        Key::DownArrow => InputKey::Down,
        Key::Home => InputKey::Home,
        Key::End => InputKey::End,
        Key::Delete => InputKey::Delete,
        Key::PageUp => InputKey::PageUp,
        Key::PageDown => InputKey::PageDown,
        Key::F1 => InputKey::F(1),
        Key::F2 => InputKey::F(2),
        Key::F3 => InputKey::F(3),
        Key::F4 => InputKey::F(4),
        Key::F5 => InputKey::F(5),
        Key::F6 => InputKey::F(6),
        Key::F7 => InputKey::F(7),
        Key::F8 => InputKey::F(8),
        Key::F9 => InputKey::F(9),
        Key::F10 => InputKey::F(10),
        Key::F11 => InputKey::F(11),
        Key::F12 => InputKey::F(12),
        _ => InputKey::Other,
    }
}
