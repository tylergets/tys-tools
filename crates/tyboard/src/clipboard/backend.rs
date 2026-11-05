use crate::clipboard::ClipboardEntry;
use crate::core::active_window::{ActiveWindowContext, ActiveWindowProvider};

pub trait ClipboardBackend {
    fn read_entry(&self) -> Option<ClipboardEntry>;
    fn read_active_window(&self) -> Option<ActiveWindowContext> {
        None
    }
}

impl<T: ClipboardBackend + ?Sized> ClipboardBackend for Box<T> {
    fn read_entry(&self) -> Option<ClipboardEntry> {
        (**self).read_entry()
    }
    fn read_active_window(&self) -> Option<ActiveWindowContext> {
        (**self).read_active_window()
    }
}

#[cfg(target_os = "linux")]
pub struct GtkClipboardBackend {
    clipboard: gtk::Clipboard,
    active_window_provider: Box<dyn ActiveWindowProvider>,
}

#[cfg(target_os = "linux")]
impl GtkClipboardBackend {
    pub fn new(
        clipboard: &gtk::Clipboard,
        active_window_provider: Box<dyn ActiveWindowProvider>,
    ) -> Self {
        Self {
            clipboard: clipboard.clone(),
            active_window_provider,
        }
    }
}

#[cfg(target_os = "linux")]
impl ClipboardBackend for GtkClipboardBackend {
    fn read_entry(&self) -> Option<ClipboardEntry> {
        if let Some(text) = self.clipboard.wait_for_text() {
            let value = text.to_string();
            if !value.is_empty() {
                return Some(ClipboardEntry::Text {
                    value,
                    source_window: None,
                });
            }
        }

        image_entry(&self.clipboard.wait_for_image()?)
    }

    fn read_active_window(&self) -> Option<ActiveWindowContext> {
        self.active_window_provider.capture()
    }
}

/// Wayland: GTK can't read the clipboard without a focused window, so watch it through
/// wl-clipboard instead.
#[cfg(target_os = "linux")]
pub struct WlClipboardBackend {
    watcher: ty_common::clipboard::ClipboardWatcher,
    active_window_provider: Box<dyn ActiveWindowProvider>,
}

#[cfg(target_os = "linux")]
impl WlClipboardBackend {
    /// `waker` fires whenever the clipboard changes; call `read_entry` then.
    pub fn new(
        active_window_provider: Box<dyn ActiveWindowProvider>,
        waker: ty_common::wake::Waker,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            watcher: ty_common::clipboard::ClipboardWatcher::start(waker)?,
            active_window_provider,
        })
    }
}

#[cfg(target_os = "linux")]
impl ClipboardBackend for WlClipboardBackend {
    fn read_entry(&self) -> Option<ClipboardEntry> {
        match self.watcher.poll()? {
            ty_common::clipboard::Content::Text(value) if !value.is_empty() => {
                Some(ClipboardEntry::Text {
                    value,
                    source_window: None,
                })
            }
            ty_common::clipboard::Content::Text(_) => None,
            ty_common::clipboard::Content::Image(image) => image_entry(&image),
        }
    }

    fn read_active_window(&self) -> Option<ActiveWindowContext> {
        self.active_window_provider.capture()
    }
}

#[cfg(target_os = "linux")]
fn image_entry(image: &gtk::gdk_pixbuf::Pixbuf) -> Option<ClipboardEntry> {
    let pixels = image.pixel_bytes()?.as_ref().to_vec();
    if pixels.is_empty() {
        return None;
    }
    Some(ClipboardEntry::Image {
        width: image.width(),
        height: image.height(),
        rowstride: image.rowstride(),
        has_alpha: image.has_alpha(),
        bits_per_sample: image.bits_per_sample(),
        channels: image.n_channels(),
        pixels,
        source_window: None,
    })
}

/// One-line label for a history entry: newlines escaped, text truncated to `char_limit`.
pub fn menu_label(entry: &ClipboardEntry, char_limit: usize) -> String {
    match entry {
        ClipboardEntry::Text { value, .. } => {
            let sanitized = value.replace('\n', "\\n").replace('\r', "\\r");
            if sanitized.chars().count() <= char_limit {
                return sanitized;
            }
            let truncated: String = sanitized.chars().take(char_limit).collect();
            format!("{truncated}...")
        }
        ClipboardEntry::Image { width, height, .. } => format!("[image] {width}x{height}"),
    }
}

/// Put `entry` on the clipboard (on X11, also handing it to the clipboard manager so it
/// outlives us). Must be called on the GTK thread.
#[cfg(target_os = "linux")]
pub fn set_clipboard_value(entry: &ClipboardEntry) -> anyhow::Result<()> {
    match entry {
        ClipboardEntry::Text { value, .. } => ty_common::clipboard::set_text(value),
        ClipboardEntry::Image {
            width,
            height,
            rowstride,
            has_alpha,
            bits_per_sample,
            pixels,
            ..
        } => {
            let bytes = gtk::glib::Bytes::from(pixels.as_slice());
            let image = gtk::gdk_pixbuf::Pixbuf::from_bytes(
                &bytes,
                gtk::gdk_pixbuf::Colorspace::Rgb,
                *has_alpha,
                *bits_per_sample,
                *width,
                *height,
                *rowstride,
            );
            ty_common::clipboard::set_image(&image)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::menu_label;
    use crate::clipboard::ClipboardEntry;

    fn text(value: &str) -> ClipboardEntry {
        ClipboardEntry::Text {
            value: value.to_string(),
            source_window: None,
        }
    }

    #[test]
    fn escapes_newlines() {
        assert_eq!(menu_label(&text("a\nb\r"), 70), "a\\nb\\r");
    }

    #[test]
    fn truncates_long_text() {
        assert_eq!(menu_label(&text("abcdef"), 3), "abc...");
        assert_eq!(menu_label(&text("abc"), 3), "abc");
    }
}
