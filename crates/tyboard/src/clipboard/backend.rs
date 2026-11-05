use crate::clipboard::ClipboardEntry;
use crate::core::active_window::{ActiveWindowContext, ActiveWindowProvider};

pub trait ClipboardBackend {
    fn read_entry(&self) -> Option<ClipboardEntry>;
    fn read_active_window(&self) -> Option<ActiveWindowContext> {
        None
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

        let image = self.clipboard.wait_for_image()?;
        let pixel_bytes = image.pixel_bytes()?;
        let pixels = pixel_bytes.as_ref().to_vec();
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

    fn read_active_window(&self) -> Option<ActiveWindowContext> {
        self.active_window_provider.capture()
    }
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

/// Put `entry` on `clipboard` and hand it to the clipboard manager so it outlives us.
#[cfg(target_os = "linux")]
pub fn set_clipboard_value(clipboard: &gtk::Clipboard, entry: &ClipboardEntry) {
    match entry {
        ClipboardEntry::Text { value, .. } => clipboard.set_text(value),
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
            clipboard.set_image(&image);
        }
    }
    clipboard.store();
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
