//! Shared appearance settings: the top-level `appearance:` section every tool's config
//! accepts. Like `env:`, it is split off by [`crate::config`] before the tool's own config is
//! parsed, and the latest loaded value is kept here for the tool to read.
//!
//! ```yaml
//! appearance:
//!   font: JetBrains Mono   # font family; unset follows the GTK font
//! ```
//!
//! Text the tools draw themselves (cairo) uses [`select_font`]; GTK widgets follow the GTK
//! font already, and [`apply_gtk`] overrides them only when `font` is set.

use std::sync::RwLock;

use anyhow::{bail, Result};
use serde::Deserialize;

/// The family cairo falls back to when neither the config nor GTK names one.
pub const DEFAULT_FONT: &str = "Sans";

/// The `appearance:` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    /// Font family for the text the tools draw and their GTK widgets. `None` follows the
    /// GTK font (`gtk-font-name`).
    pub font: Option<String>,
}

impl Appearance {
    pub fn validate(&self) -> Result<()> {
        if self
            .font
            .as_deref()
            .is_some_and(|font| font.trim().is_empty())
        {
            bail!("appearance.font cannot be empty");
        }
        Ok(())
    }
}

static CURRENT: RwLock<Appearance> = RwLock::new(Appearance { font: None });

/// Replace the current settings (done by [`crate::config::read`] on every load).
pub fn set(appearance: Appearance) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = appearance;
}

/// The settings from the last loaded config (defaults before any load).
pub fn current() -> Appearance {
    CURRENT.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The font family to draw text with: the config's `appearance.font`, else the GTK font
/// (on the GTK thread), else [`DEFAULT_FONT`].
pub fn font_family() -> String {
    if let Some(font) = current().font {
        return font.trim().to_string();
    }
    gtk_font_family().unwrap_or_else(|| DEFAULT_FONT.to_string())
}

/// The family of GTK's `gtk-font-name` (e.g. `"Cantarell"` from `"Cantarell 11"`), when
/// GTK is initialized on this thread.
#[cfg(target_os = "linux")]
fn gtk_font_family() -> Option<String> {
    use gtk::prelude::*;
    if !gtk::is_initialized_main_thread() {
        return None;
    }
    let name = gtk::Settings::default()?.gtk_font_name()?;
    let family = gtk::pango::FontDescription::from_string(&name).family()?;
    let family = family.trim();
    (!family.is_empty()).then(|| family.to_string())
}

#[cfg(not(target_os = "linux"))]
fn gtk_font_family() -> Option<String> {
    None
}

/// Select the shared font on a cairo context, keeping the given weight. Replaces
/// `cr.select_font_face("Sans", ...)`; set the size afterwards as before.
#[cfg(target_os = "linux")]
pub fn select_font(cr: &gtk::cairo::Context, weight: gtk::cairo::FontWeight) {
    cr.select_font_face(&font_family(), gtk::cairo::FontSlant::Normal, weight);
}

/// Make GTK widgets use `appearance.font` when it is set. Call once after `gtk::init`
/// ([`crate::tray::init_gtk`] does); without a font set, GTK's own font applies.
#[cfg(target_os = "linux")]
pub fn apply_gtk() {
    use gtk::prelude::*;
    let Some(font) = current().font else {
        return;
    };
    let Some(screen) = gtk::gdk::Screen::default() else {
        return;
    };
    let css = format!(
        "* {{ font-family: \"{}\"; }}",
        font.trim().replace('\\', "\\\\").replace('"', "\\\"")
    );
    let provider = gtk::CssProvider::new();
    if let Err(err) = provider.load_from_data(css.as_bytes()) {
        eprintln!("warning: ignoring appearance.font {font:?}: {err}");
        return;
    }
    gtk::StyleContext::add_provider_for_screen(
        &screen,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates() {
        let appearance: Appearance = serde_yaml_ng::from_str("font: Iosevka").unwrap();
        assert_eq!(appearance.font.as_deref(), Some("Iosevka"));
        assert!(appearance.validate().is_ok());
        assert!(Appearance::default().validate().is_ok());
        let blank = Appearance {
            font: Some("  ".into()),
        };
        assert!(blank.validate().is_err());
        assert!(serde_yaml_ng::from_str::<Appearance>("fnot: x").is_err());
    }

    #[test]
    fn config_font_wins() {
        set(Appearance {
            font: Some(" Iosevka ".into()),
        });
        assert_eq!(font_family(), "Iosevka");
        set(Appearance::default());
        assert!(!font_family().is_empty());
    }
}
