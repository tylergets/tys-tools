//! Delivering transcribed text: typing it into the focused window, or the clipboard.

use std::process::Command;

use anyhow::{bail, Context, Result};
use gtk::gdk;

/// Type `text` into the focused window with xdotool. Blocks until typing finishes.
pub fn type_text(text: &str, delay_ms: u32) -> Result<()> {
    let output = Command::new("xdotool")
        .args(["type", "--clearmodifiers", "--delay"])
        .arg(delay_ms.to_string())
        .arg("--")
        .arg(text)
        .output()
        .context("failed to run xdotool")?;
    if !output.status.success() {
        bail!(
            "xdotool failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Put `text` on the clipboard. Must be called on the GTK thread; the clipboard is served for
/// as long as tyspeak runs (and handed to a clipboard manager on exit, if one runs).
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    let display = gdk::Display::default().context("no display for the clipboard")?;
    let clipboard = gtk::Clipboard::default(&display).context("no clipboard")?;
    clipboard.set_text(text);
    clipboard.store();
    Ok(())
}
