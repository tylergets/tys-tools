//! Shared plumbing for the ty* desktop tools (tyboard, tycapture, tykey, tyshot, tyspeak, tylight, tygesture, typort, tyci).
//!
//! The display layer (`session`, `screen`, `wm`, `overlay`, `clipboard`, `input`) hides the
//! difference between X11 and Wayland, so the tools themselves don't need to care.

#[cfg(target_os = "linux")]
pub mod clipboard;
pub mod config;
#[cfg(target_os = "linux")]
pub mod frame;
pub mod geometry;
#[cfg(target_os = "linux")]
pub mod icons;
#[cfg(target_os = "linux")]
pub mod input;
pub mod lock;
pub mod notify;
#[cfg(target_os = "linux")]
pub mod overlay;
#[cfg(target_os = "linux")]
pub mod screen;
pub mod session;
pub mod status;
#[cfg(target_os = "linux")]
pub mod tray;
#[cfg(target_os = "linux")]
mod tray_sni;
pub mod wake;
#[cfg(target_os = "linux")]
pub mod wm;

/// `$USER`, or `"user"` if unset. Used to keep per-user runtime files apart.
pub fn user_hint() -> String {
    std::env::var("USER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "user".to_string())
}

/// `$XDG_RUNTIME_DIR`, or the temp dir if unset.
pub fn runtime_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}
