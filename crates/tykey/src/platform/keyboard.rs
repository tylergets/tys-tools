//! The keyboard backend: global key listening and typed output, from ty-common's input layer
//! (rdev/enigo on X11; `/dev/input` and wtype on Wayland).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ty_common::input::{InputKey, Key, KeyInput, Keyboard};

use crate::core::expansion::OutputAction;
use crate::io::events::{KeyEvent, KeyEventKind, SpecialInputKey};
use crate::io::output::{OutputSink, SpecialKey};

pub struct KeyboardBackend {
    /// Set while we type, so our own key events are not treated as user input.
    injecting: Arc<AtomicBool>,
    keyboard: Keyboard,
}

impl KeyboardBackend {
    pub fn new() -> Result<Self> {
        Ok(Self {
            injecting: Arc::new(AtomicBool::new(false)),
            keyboard: Keyboard::new(None)?,
        })
    }

    /// Listen for key events until the listener fails. Blocks.
    pub fn listen<F>(&self, mut on_event: F) -> Result<()>
    where
        F: FnMut(KeyEvent) + Send + 'static,
    {
        let injecting = Arc::clone(&self.injecting);
        ty_common::input::listen(move |input| {
            on_event(map_event(input, injecting.load(Ordering::Relaxed)));
        })
    }

    /// Run `send` with the injecting flag set, clearing it afterwards even on error.
    fn injecting(&self, send: impl FnOnce() -> Result<()>) -> Result<()> {
        self.injecting.store(true, Ordering::Relaxed);
        let result = send();
        self.injecting.store(false, Ordering::Relaxed);
        result
    }
}

impl OutputSink for KeyboardBackend {
    fn send_backspaces(&self, count: usize) -> Result<()> {
        self.injecting(|| self.keyboard.tap(Key::Backspace, count))
    }

    fn send_actions(&self, actions: &[OutputAction]) -> Result<()> {
        self.injecting(|| {
            for action in actions {
                match action {
                    OutputAction::Text(text) => self.keyboard.type_text(text)?,
                    OutputAction::Key(key) => self.keyboard.tap(map_special_key(*key), 1)?,
                    OutputAction::SleepMs(ms) => std::thread::sleep(Duration::from_millis(*ms)),
                    OutputAction::MoveCaret(amount) => {
                        let key = if *amount < 0 { Key::Left } else { Key::Right };
                        self.keyboard.tap(key, amount.unsigned_abs() as usize)?;
                    }
                }
            }
            Ok(())
        })
    }
}

fn map_event(input: KeyInput, is_injected: bool) -> KeyEvent {
    KeyEvent {
        kind: if input.pressed {
            KeyEventKind::Press
        } else {
            KeyEventKind::Release
        },
        printable: input.text,
        special: Some(map_input_key(input.key)),
        is_injected,
    }
}

fn map_input_key(key: InputKey) -> SpecialInputKey {
    match key {
        InputKey::Shift => SpecialInputKey::Shift,
        InputKey::Ctrl => SpecialInputKey::Ctrl,
        InputKey::Alt => SpecialInputKey::Alt,
        InputKey::Meta => SpecialInputKey::Meta,
        InputKey::CapsLock => SpecialInputKey::CapsLock,
        InputKey::Enter => SpecialInputKey::Enter,
        InputKey::Tab => SpecialInputKey::Tab,
        InputKey::Backspace => SpecialInputKey::Backspace,
        InputKey::Escape => SpecialInputKey::Escape,
        InputKey::Left => SpecialInputKey::Left,
        InputKey::Right => SpecialInputKey::Right,
        InputKey::Up => SpecialInputKey::Up,
        InputKey::Down => SpecialInputKey::Down,
        InputKey::Home => SpecialInputKey::Home,
        InputKey::End => SpecialInputKey::End,
        InputKey::Delete => SpecialInputKey::Delete,
        InputKey::PageUp => SpecialInputKey::PageUp,
        InputKey::PageDown => SpecialInputKey::PageDown,
        InputKey::F(1) => SpecialInputKey::F1,
        InputKey::F(2) => SpecialInputKey::F2,
        InputKey::F(3) => SpecialInputKey::F3,
        InputKey::F(4) => SpecialInputKey::F4,
        InputKey::F(5) => SpecialInputKey::F5,
        InputKey::F(6) => SpecialInputKey::F6,
        InputKey::F(7) => SpecialInputKey::F7,
        InputKey::F(8) => SpecialInputKey::F8,
        InputKey::F(9) => SpecialInputKey::F9,
        InputKey::F(10) => SpecialInputKey::F10,
        InputKey::F(11) => SpecialInputKey::F11,
        InputKey::F(12) => SpecialInputKey::F12,
        InputKey::F(_) | InputKey::Other => SpecialInputKey::Unknown,
    }
}

fn map_special_key(key: SpecialKey) -> Key {
    match key {
        SpecialKey::Enter => Key::Enter,
        SpecialKey::Tab => Key::Tab,
        SpecialKey::Escape => Key::Escape,
        SpecialKey::Backspace => Key::Backspace,
        SpecialKey::Space => Key::Space,
        SpecialKey::Left => Key::Left,
        SpecialKey::Right => Key::Right,
        SpecialKey::Up => Key::Up,
        SpecialKey::Down => Key::Down,
        SpecialKey::Home => Key::Home,
        SpecialKey::End => Key::End,
        SpecialKey::Delete => Key::Delete,
        SpecialKey::PageUp => Key::PageUp,
        SpecialKey::PageDown => Key::PageDown,
        SpecialKey::F1 => Key::F(1),
        SpecialKey::F2 => Key::F(2),
        SpecialKey::F3 => Key::F(3),
        SpecialKey::F4 => Key::F(4),
        SpecialKey::F5 => Key::F(5),
        SpecialKey::F6 => Key::F(6),
        SpecialKey::F7 => Key::F(7),
        SpecialKey::F8 => Key::F(8),
        SpecialKey::F9 => Key::F(9),
        SpecialKey::F10 => Key::F(10),
        SpecialKey::F11 => Key::F(11),
        SpecialKey::F12 => Key::F(12),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_press_with_text() {
        let event = map_event(
            KeyInput {
                pressed: true,
                key: InputKey::Other,
                text: Some('a'),
            },
            false,
        );
        assert_eq!(event.kind, KeyEventKind::Press);
        assert_eq!(event.printable, Some('a'));
        assert_eq!(event.special, Some(SpecialInputKey::Unknown));
    }

    #[test]
    fn maps_function_keys_both_ways() {
        assert_eq!(map_input_key(InputKey::F(5)), SpecialInputKey::F5);
        assert_eq!(map_special_key(SpecialKey::F12), Key::F(12));
    }
}
