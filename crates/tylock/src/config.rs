use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use ty_common::config::Loaded;

pub const APP_NAME: &str = "tylock";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Image behind the clock, scaled to cover each monitor. Unset: `~/.background-image`
    /// when it exists, else `colors.background`.
    pub wallpaper: Option<PathBuf>,
    /// Blur radius for the wallpaper, in pixels on a 1080p screen (scaled with the
    /// resolution). 0 turns blurring off.
    pub blur: f64,
    /// How much to darken the wallpaper, from 0 (not at all) to 1 (black).
    pub dim: f64,
    /// strftime formats (chrono) for the clock and the date under it.
    pub time_format: String,
    pub date_format: String,
    /// Font family for the clock. Unset: `appearance.font`.
    pub clock_font: Option<String>,
    /// Unlock with a finger too, when fprintd has prints for you.
    pub fingerprint: bool,
    /// The PAM service that checks passwords. It must not include pam_fprintd, or a
    /// password check waits for the reader first.
    pub pam_service: String,
    /// The PAM service for fingers: pam_fprintd only.
    pub fingerprint_pam_service: String,
    pub colors: Colors,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            wallpaper: None,
            blur: 0.0,
            dim: 0.3,
            time_format: "%H:%M".into(),
            date_format: "%A, %B %-d".into(),
            clock_font: None,
            fingerprint: true,
            pam_service: "tylock".into(),
            fingerprint_pam_service: "tylock-fingerprint".into(),
            colors: Colors::default(),
        }
    }
}

/// `#rrggbb` or `#rrggbbaa`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    /// The clock and other text.
    pub text: String,
    /// Hints and secondary text.
    pub muted: String,
    /// The password field's border and the fingerprint while waiting.
    pub accent: String,
    /// A wrong password or finger.
    pub error: String,
    /// A matching finger.
    pub success: String,
    /// The Caps Lock warning.
    pub warning: String,
    /// The password field and fingerprint button, and the screen when there is no
    /// wallpaper.
    pub background: String,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            text: "#f2f2f2".into(),
            muted: "#c0c0c0".into(),
            accent: "#a3b8d8".into(),
            error: "#e99089".into(),
            success: "#a7d7b0".into(),
            warning: "#e7d39a".into(),
            background: "#242424".into(),
        }
    }
}

/// An RGBA color, 0–1 per channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgba {
    pub fn parse(text: &str) -> Result<Self> {
        let hex = text
            .strip_prefix('#')
            .with_context(|| format!("color {text:?} must start with #"))?;
        if !(hex.len() == 6 || hex.len() == 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("color {text:?} must be #rrggbb or #rrggbbaa");
        }
        let channel = |i: usize| f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap()) / 255.0;
        Ok(Self {
            r: channel(0),
            g: channel(2),
            b: channel(4),
            a: if hex.len() == 8 { channel(6) } else { 1.0 },
        })
    }
}

/// [`Colors`], parsed.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub text: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub error: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub background: Rgba,
}

impl Colors {
    pub fn palette(&self) -> Result<Palette> {
        let parse = |name: &str, value: &str| {
            Rgba::parse(value).with_context(|| format!("in colors.{name}"))
        };
        Ok(Palette {
            text: parse("text", &self.text)?,
            muted: parse("muted", &self.muted)?,
            accent: parse("accent", &self.accent)?,
            error: parse("error", &self.error)?,
            success: parse("success", &self.success)?,
            warning: parse("warning", &self.warning)?,
            background: parse("background", &self.background)?,
        })
    }
}

impl AppConfig {
    pub fn load(path: Option<PathBuf>) -> Result<Loaded<Self>> {
        ty_common::config::load(APP_NAME, path)
    }

    pub fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.dim) {
            bail!("dim must be between 0 and 1");
        }
        if !(0.0..=100.0).contains(&self.blur) {
            bail!("blur must be between 0 and 100");
        }
        for (name, format) in [
            ("time_format", &self.time_format),
            ("date_format", &self.date_format),
        ] {
            let items = chrono::format::StrftimeItems::new(format);
            if items
                .into_iter()
                .any(|item| matches!(item, chrono::format::Item::Error))
            {
                bail!("{name} {format:?} is not a valid strftime format");
            }
        }
        if self.pam_service.trim().is_empty() {
            bail!("pam_service cannot be empty");
        }
        if self.fingerprint && self.fingerprint_pam_service.trim().is_empty() {
            bail!("fingerprint_pam_service cannot be empty");
        }
        if self.fingerprint && self.fingerprint_pam_service == self.pam_service {
            bail!("fingerprint_pam_service must be a different PAM service than pam_service");
        }
        self.colors.palette()?;
        Ok(())
    }

    /// The wallpaper to load: the configured one, else `~/.background-image` if present.
    pub fn wallpaper_path(&self) -> Option<PathBuf> {
        match &self.wallpaper {
            Some(path) => Some(ty_common::config::expand_user(path)),
            None => dirs::home_dir()
                .map(|home| home.join(".background-image"))
                .filter(|path| path.exists()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_config_is_default() {
        let config: AppConfig = ty_common::config::parse("").unwrap();
        assert_eq!(config, AppConfig::default());
        config.validate().unwrap();
    }

    #[test]
    fn parses_config() {
        let config: AppConfig = ty_common::config::parse(
            "wallpaper: ~/wall.png\nblur: 8\ndim: 0.5\ncolors:\n  accent: '#112233aa'\n",
        )
        .unwrap();
        config.validate().unwrap();
        assert_eq!(config.blur, 8.0);
        let accent = config.colors.palette().unwrap().accent;
        assert!((accent.a - 170.0 / 255.0).abs() < 1e-9);
        assert!(config.wallpaper_path().unwrap().ends_with("wall.png"));
    }

    #[test]
    fn rejects_bad_values() {
        let bad = |yaml: &str| {
            let config: AppConfig = ty_common::config::parse(yaml).unwrap();
            assert!(config.validate().is_err(), "{yaml}");
        };
        bad("dim: 2");
        bad("colors: { text: red }");
        bad("colors: { text: '#12345' }");
        bad("time_format: '%Q'");
        bad("fingerprint_pam_service: tylock");
        assert!(ty_common::config::parse::<AppConfig>("nope: 1").is_err());
    }
}
