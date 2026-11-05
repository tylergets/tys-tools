use anyhow::Result;

use crate::core::expansion::OutputAction;

#[derive(Debug, Clone, Copy)]
pub enum SpecialKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Delete,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

pub trait OutputSink: Send + Sync {
    fn send_backspaces(&self, count: usize) -> Result<()>;
    fn send_actions(&self, actions: &[OutputAction]) -> Result<()>;
}
