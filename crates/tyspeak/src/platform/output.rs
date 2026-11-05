//! Delivering transcribed text: typing it into the focused window, or the clipboard.

use anyhow::Result;

/// Type `text` into the focused window (xdotool on X11, wtype on Wayland). Blocks until
/// typing finishes.
pub fn type_text(text: &str, delay_ms: u32) -> Result<()> {
    ty_common::input::type_text(text, delay_ms)
}

/// Put `text` on the clipboard. Must be called on the GTK thread. On X11 the clipboard is
/// served for as long as tyspeak runs (and handed to a clipboard manager on exit, if one
/// runs); on Wayland `wl-copy` serves it.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    ty_common::clipboard::set_text(text)
}
