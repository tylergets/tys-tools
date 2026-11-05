use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use enigo::{Direction, Enigo, Key as EnigoKey, Keyboard, Settings};
use rdev::{Event, EventType, Key};

use crate::core::expansion::OutputAction;
use crate::io::events::{KeyEvent, KeyEventKind, SpecialInputKey};
use crate::io::output::{OutputSink, SpecialKey};

pub struct X11RdevBackend {
    injecting: Arc<AtomicBool>,
    enigo: Mutex<Enigo>,
}

impl X11RdevBackend {
    pub fn new() -> Result<Self> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|err| anyhow::anyhow!("failed to initialize enigo: {err}"))?;
        Ok(Self {
            injecting: Arc::new(AtomicBool::new(false)),
            enigo: Mutex::new(enigo),
        })
    }

    pub fn listen<F>(&self, mut on_event: F) -> Result<()>
    where
        F: FnMut(KeyEvent) + Send + 'static,
    {
        let injecting_flag = Arc::clone(&self.injecting);

        rdev::listen(move |event| {
            if let Some(mapped) = map_event(&event, injecting_flag.load(Ordering::Relaxed)) {
                on_event(mapped);
            }
        })
        .map_err(|err| anyhow::anyhow!("failed to start global X11 listener: {err:?}"))
    }
}

impl OutputSink for X11RdevBackend {
    fn send_backspaces(&self, count: usize) -> Result<()> {
        self.injecting.store(true, Ordering::Relaxed);
        let mut enigo = self.enigo.lock().expect("enigo mutex poisoned");
        for _ in 0..count {
            tap_key(&mut enigo, EnigoKey::Backspace)?;
        }
        self.injecting.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn send_actions(&self, actions: &[OutputAction]) -> Result<()> {
        self.injecting.store(true, Ordering::Relaxed);
        let mut enigo = self.enigo.lock().expect("enigo mutex poisoned");
        for action in actions {
            match action {
                OutputAction::Text(s) => {
                    enigo
                        .text(s)
                        .map_err(|err| anyhow::anyhow!("text simulation failed: {err}"))?;
                    std::thread::sleep(Duration::from_millis(1));
                }
                OutputAction::Key(k) => tap_key(&mut enigo, map_special_key(*k))?,
                OutputAction::SleepMs(ms) => {
                    std::thread::sleep(Duration::from_millis(*ms));
                }
                OutputAction::MoveCaret(amount) => {
                    let key = if *amount < 0 {
                        EnigoKey::LeftArrow
                    } else {
                        EnigoKey::RightArrow
                    };
                    for _ in 0..amount.unsigned_abs() {
                        tap_key(&mut enigo, key)?;
                    }
                }
            }
        }
        self.injecting.store(false, Ordering::Relaxed);
        Ok(())
    }
}

fn tap_key(enigo: &mut Enigo, key: EnigoKey) -> Result<()> {
    enigo
        .key(key, Direction::Press)
        .map_err(|err| anyhow::anyhow!("key press simulation failed: {err}"))?;
    std::thread::sleep(Duration::from_millis(1));
    enigo
        .key(key, Direction::Release)
        .map_err(|err| anyhow::anyhow!("key release simulation failed: {err}"))?;
    std::thread::sleep(Duration::from_millis(1));
    Ok(())
}

fn map_event(event: &Event, is_injected: bool) -> Option<KeyEvent> {
    match event.event_type {
        EventType::KeyPress(key) => Some(KeyEvent {
            kind: KeyEventKind::Press,
            printable: event.name.as_deref().and_then(extract_single_char),
            special: Some(map_input_key(key)),
            is_injected,
        }),
        EventType::KeyRelease(key) => Some(KeyEvent {
            kind: KeyEventKind::Release,
            printable: None,
            special: Some(map_input_key(key)),
            is_injected,
        }),
        _ => None,
    }
}

fn extract_single_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(first)
}

fn map_input_key(key: Key) -> SpecialInputKey {
    match key {
        Key::ShiftLeft | Key::ShiftRight => SpecialInputKey::Shift,
        Key::ControlLeft | Key::ControlRight => SpecialInputKey::Ctrl,
        Key::Alt | Key::AltGr => SpecialInputKey::Alt,
        Key::MetaLeft | Key::MetaRight => SpecialInputKey::Meta,
        Key::CapsLock => SpecialInputKey::CapsLock,
        Key::Return => SpecialInputKey::Enter,
        Key::Tab => SpecialInputKey::Tab,
        Key::Backspace => SpecialInputKey::Backspace,
        Key::Escape => SpecialInputKey::Escape,
        Key::LeftArrow => SpecialInputKey::Left,
        Key::RightArrow => SpecialInputKey::Right,
        Key::UpArrow => SpecialInputKey::Up,
        Key::DownArrow => SpecialInputKey::Down,
        Key::Home => SpecialInputKey::Home,
        Key::End => SpecialInputKey::End,
        Key::Delete => SpecialInputKey::Delete,
        Key::PageUp => SpecialInputKey::PageUp,
        Key::PageDown => SpecialInputKey::PageDown,
        Key::F1 => SpecialInputKey::F1,
        Key::F2 => SpecialInputKey::F2,
        Key::F3 => SpecialInputKey::F3,
        Key::F4 => SpecialInputKey::F4,
        Key::F5 => SpecialInputKey::F5,
        Key::F6 => SpecialInputKey::F6,
        Key::F7 => SpecialInputKey::F7,
        Key::F8 => SpecialInputKey::F8,
        Key::F9 => SpecialInputKey::F9,
        Key::F10 => SpecialInputKey::F10,
        Key::F11 => SpecialInputKey::F11,
        Key::F12 => SpecialInputKey::F12,
        _ => SpecialInputKey::Unknown,
    }
}

fn map_special_key(key: SpecialKey) -> EnigoKey {
    match key {
        SpecialKey::Enter => EnigoKey::Return,
        SpecialKey::Tab => EnigoKey::Tab,
        SpecialKey::Escape => EnigoKey::Escape,
        SpecialKey::Backspace => EnigoKey::Backspace,
        SpecialKey::Space => EnigoKey::Space,
        SpecialKey::Left => EnigoKey::LeftArrow,
        SpecialKey::Right => EnigoKey::RightArrow,
        SpecialKey::Up => EnigoKey::UpArrow,
        SpecialKey::Down => EnigoKey::DownArrow,
        SpecialKey::Home => EnigoKey::Home,
        SpecialKey::End => EnigoKey::End,
        SpecialKey::Delete => EnigoKey::Delete,
        SpecialKey::PageUp => EnigoKey::PageUp,
        SpecialKey::PageDown => EnigoKey::PageDown,
        SpecialKey::F1 => EnigoKey::F1,
        SpecialKey::F2 => EnigoKey::F2,
        SpecialKey::F3 => EnigoKey::F3,
        SpecialKey::F4 => EnigoKey::F4,
        SpecialKey::F5 => EnigoKey::F5,
        SpecialKey::F6 => EnigoKey::F6,
        SpecialKey::F7 => EnigoKey::F7,
        SpecialKey::F8 => EnigoKey::F8,
        SpecialKey::F9 => EnigoKey::F9,
        SpecialKey::F10 => EnigoKey::F10,
        SpecialKey::F11 => EnigoKey::F11,
        SpecialKey::F12 => EnigoKey::F12,
    }
}
