//! Putting a PNG on the X11 clipboard.
//!
//! An X11 clipboard is served live by the program that owns it, so the data disappears when
//! that program exits. Copying hands the PNG to a detached `tyshot serve-clipboard` helper:
//! the helper takes ownership, gives the data to a clipboard manager if one is running
//! (`CLIPBOARD_MANAGER` / `SAVE_TARGETS`) and exits, or otherwise keeps serving it until
//! another program takes the clipboard. `tyshot` itself returns as soon as the helper owns it.

use std::cell::Cell;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use gtk::glib::translate::ToGlibPtr;

/// Hidden subcommand that runs [`serve`].
pub const SERVE_COMMAND: &str = "serve-clipboard";
/// The helper's first stdout line once it owns the clipboard. Anything else is an error.
const READY: &str = "ready";
const PNG_TARGET: &str = "image/png";

/// Put `png` on the clipboard. Returns once the helper owns the clipboard.
pub fn copy_png(png: &[u8]) -> Result<()> {
    let exe = std::env::current_exe().context("failed to find the tyshot executable")?;
    let mut child = Command::new(exe)
        .arg(SERVE_COMMAND)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Out of the caller's process group, so Ctrl+C in the terminal doesn't clear it.
        .process_group(0)
        .spawn()
        .context("failed to start the clipboard helper")?;

    // Dropping stdin after writing sends EOF.
    child
        .stdin
        .take()
        .context("no stdin for the clipboard helper")?
        .write_all(png)
        .context("failed to send the image to the clipboard helper")?;

    let mut line = String::new();
    BufReader::new(
        child
            .stdout
            .take()
            .context("no stdout for the clipboard helper")?,
    )
    .read_line(&mut line)
    .context("failed to read from the clipboard helper")?;
    match line.trim() {
        READY => Ok(()),
        "" => bail!("clipboard helper exited ({})", child.wait()?),
        message => bail!("clipboard helper: {message}"),
    }
}

thread_local! {
    static SERVING: Cell<bool> = const { Cell::new(false) };
}

/// Dropped by GTK when another program takes the clipboard; ends the helper.
struct OwnershipGuard;

impl Drop for OwnershipGuard {
    fn drop(&mut self) {
        if SERVING.with(Cell::get) && gtk::main_level() > 0 {
            gtk::main_quit();
        }
    }
}

/// Body of the `serve-clipboard` helper: read a PNG from stdin and serve it.
pub fn serve() -> Result<()> {
    let mut stdout = std::io::stdout();
    let clipboard = match take_clipboard() {
        Ok(clipboard) => clipboard,
        Err(err) => {
            let _ = writeln!(stdout, "{err:#}");
            return Err(err);
        }
    };
    let _ = writeln!(stdout, "{READY}");
    let _ = stdout.flush();

    let persistent = clipboard
        .display()
        .is_some_and(|display| display.supports_clipboard_persistence());
    if persistent {
        // SAFETY: a valid clipboard; NULL targets means "store every target".
        unsafe {
            gtk::ffi::gtk_clipboard_set_can_store(clipboard.to_glib_none().0, std::ptr::null(), 0);
        }
        // Blocks until the clipboard manager has copied the data (or gives up).
        clipboard.store();
        return Ok(());
    }

    SERVING.with(|serving| serving.set(true));
    gtk::main();
    Ok(())
}

fn take_clipboard() -> Result<gtk::Clipboard> {
    let mut png = Vec::new();
    std::io::stdin()
        .read_to_end(&mut png)
        .context("failed to read the image")?;
    if png.is_empty() {
        bail!("no image data");
    }

    gtk::init().context("failed to initialize GTK")?;
    let display = gtk::gdk::Display::default().context("no display for the clipboard")?;
    let clipboard = gtk::Clipboard::default(&display).context("no clipboard")?;
    let targets = [gtk::TargetEntry::new(
        PNG_TARGET,
        gtk::TargetFlags::empty(),
        0,
    )];
    let guard = OwnershipGuard;
    let owned = clipboard.set_with_data(&targets, move |_, selection, _| {
        let _ = &guard;
        selection.set(&selection.target(), 8, &png);
    });
    if !owned {
        bail!("failed to take the clipboard");
    }
    Ok(clipboard)
}
