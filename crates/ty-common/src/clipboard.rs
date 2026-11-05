//! The clipboard, on X11 (through GTK) and Wayland (through `wl-copy`/`wl-paste`).
//!
//! GTK can't use the Wayland clipboard without a focused window, which tray daemons never
//! have, so on Wayland everything goes through wl-clipboard (the data-control protocol,
//! supported by wlroots compositors and KDE).
//!
//! An X11 clipboard is served live by the program that owns it, so its data disappears when
//! that program exits. [`copy_png_detached`] hands the PNG to a detached helper (the calling
//! tool's hidden [`SERVE_COMMAND`] subcommand, which must call [`serve`]): the helper takes
//! ownership, gives the data to a clipboard manager if one is running (`CLIPBOARD_MANAGER` /
//! `SAVE_TARGETS`) and exits, or otherwise keeps serving it until another program takes the
//! clipboard. `wl-copy` does the same on its own.

use std::cell::Cell;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use anyhow::{bail, Context, Result};
use gtk::gdk_pixbuf::{Pixbuf, PixbufLoader};
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;

use crate::session;

/// Hidden subcommand that runs [`serve`].
pub const SERVE_COMMAND: &str = "serve-clipboard";
/// The helper's first stdout line once it owns the clipboard. Anything else is an error.
const READY: &str = "ready";
const PNG_TARGET: &str = "image/png";
/// Text types, best first.
const TEXT_TYPES: &[&str] = &[
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];

/// Put `text` on the clipboard. On X11 this must be called on the GTK thread, and the text is
/// served for as long as the program runs (and handed to a clipboard manager on exit).
pub fn set_text(text: &str) -> Result<()> {
    if session::is_wayland() {
        return wl_copy(None, text.as_bytes());
    }
    let clipboard = gtk_clipboard()?;
    clipboard.set_text(text);
    clipboard.store();
    Ok(())
}

/// Put an image on the clipboard. Same rules as [`set_text`].
pub fn set_image(image: &Pixbuf) -> Result<()> {
    if session::is_wayland() {
        let png = image
            .save_to_bufferv("png", &[])
            .context("failed to encode the image")?;
        return wl_copy(Some(PNG_TARGET), &png);
    }
    let clipboard = gtk_clipboard()?;
    clipboard.set_image(image);
    clipboard.store();
    Ok(())
}

/// Put a PNG on the clipboard so that it stays there after this program exits. Returns once
/// the clipboard has been taken. See the module docs for how.
pub fn copy_png_detached(png: &[u8]) -> Result<()> {
    if session::is_wayland() {
        return wl_copy(Some(PNG_TARGET), png);
    }
    let exe = std::env::current_exe().context("failed to find our own executable")?;
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

/// Body of the [`SERVE_COMMAND`] helper (X11): read a PNG from stdin and serve it.
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
    let clipboard = gtk_clipboard()?;
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

fn gtk_clipboard() -> Result<gtk::Clipboard> {
    let display = gtk::gdk::Display::default().context("no display for the clipboard")?;
    gtk::Clipboard::default(&display).context("no clipboard")
}

/// `wl-copy` reads everything, then forks to serve it in the background. Its output goes
/// nowhere: the forked server would otherwise hold our pipes open and we'd wait on it.
fn wl_copy(mime: Option<&str>, data: &[u8]) -> Result<()> {
    let mut command = Command::new("wl-copy");
    if let Some(mime) = mime {
        command.args(["--type", mime]);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to run wl-copy (needed for the clipboard on Wayland)")?;
    child
        .stdin
        .take()
        .context("no stdin for wl-copy")?
        .write_all(data)
        .context("failed to send data to wl-copy")?;
    let status = child.wait()?;
    if !status.success() {
        bail!("wl-copy failed ({status})");
    }
    Ok(())
}

/// What is on the clipboard.
pub enum Content {
    Text(String),
    Image(Pixbuf),
}

/// Wayland: notices clipboard changes (via `wl-paste --watch`) and reads the new content.
pub struct ClipboardWatcher {
    child: Child,
    changed: Arc<AtomicBool>,
}

impl ClipboardWatcher {
    /// `waker` fires (from the watcher thread) each time the clipboard changes.
    pub fn start(waker: crate::wake::Waker) -> Result<Self> {
        let mut command = Command::new("wl-paste");
        // SAFETY: prctl is async-signal-safe. Ends the watcher if we die without dropping it.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let mut child = command
            .args(["--watch", "echo"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to run wl-paste (needed for the clipboard on Wayland)")?;
        let stdout = child.stdout.take().context("no stdout for wl-paste")?;
        // Start "changed" so the first poll reads whatever is already there.
        let changed = Arc::new(AtomicBool::new(true));
        waker.wake();
        {
            let changed = changed.clone();
            thread::spawn(move || {
                for _ in BufReader::new(stdout).lines().map_while(Result::ok) {
                    changed.store(true, Ordering::Relaxed);
                    waker.wake();
                }
            });
        }
        Ok(Self { child, changed })
    }

    /// The new clipboard content if it changed since the last call, else `None`.
    pub fn poll(&self) -> Option<Content> {
        if !self.changed.swap(false, Ordering::Relaxed) {
            return None;
        }
        read_wayland()
            .map_err(|err| eprintln!("warning: failed to read the clipboard: {err:#}"))
            .ok()
            .flatten()
    }
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_wayland() -> Result<Option<Content>> {
    let types = wl_paste(&["--list-types"])?;
    let types: Vec<&str> = std::str::from_utf8(&types)?.lines().collect();
    if let Some(text_type) = TEXT_TYPES.iter().find(|wanted| types.contains(wanted)) {
        let text = wl_paste(&["--no-newline", "--type", text_type])?;
        return Ok(Some(Content::Text(
            String::from_utf8_lossy(&text).into_owned(),
        )));
    }
    if types.contains(&PNG_TARGET) {
        let png = wl_paste(&["--type", PNG_TARGET])?;
        let loader = PixbufLoader::new();
        loader.write(&png)?;
        loader.close()?;
        return Ok(loader.pixbuf().map(Content::Image));
    }
    Ok(None)
}

fn wl_paste(args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("wl-paste")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("failed to run wl-paste")?;
    if !output.status.success() {
        // An empty clipboard is not an error.
        let message = String::from_utf8_lossy(&output.stderr);
        if message.contains("No selection") || message.contains("Nothing is copied") {
            return Ok(Vec::new());
        }
        bail!("wl-paste failed: {}", message.trim());
    }
    Ok(output.stdout)
}
