#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventKind {
    Press,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialInputKey {
    Enter,
    Tab,
    Backspace,
    Shift,
    Ctrl,
    Alt,
    Meta,
    CapsLock,
    Escape,
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
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub kind: KeyEventKind,
    pub printable: Option<char>,
    pub special: Option<SpecialInputKey>,
    pub is_injected: bool,
}
