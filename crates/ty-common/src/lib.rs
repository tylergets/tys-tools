//! Shared plumbing for the ty* desktop tools (tyboard, tycapture, tykey, tyshot, tyspeak, tylight).

pub mod config;
#[cfg(target_os = "linux")]
pub mod icons;
pub mod lock;
pub mod notify;
pub mod status;
#[cfg(target_os = "linux")]
pub mod tray;
#[cfg(target_os = "linux")]
mod tray_sni;

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
