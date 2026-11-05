use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use ty_common::config::expand_home;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default)]
    pub save: SaveConfig,
    #[serde(default)]
    pub notifications: NotificationsConfig,
    #[serde(default)]
    pub overlay: OverlayConfig,
    #[serde(default)]
    pub annotation: AnnotationConfig,
    #[serde(default)]
    pub toolbar: ToolbarConfig,
    #[serde(default)]
    pub opencode: OpencodeConfig,
}

/// Sending screenshots to an opencode session (Ctrl+Enter in the overlay).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OpencodeConfig {
    /// Start the agent on the screenshot right away. When false (the default) it waits in
    /// the session and is sent along with the next message typed there.
    pub submit: bool,
    /// Terminal that new sessions are opened in. Run as
    /// `<terminal> --directory <dir> opencode --session <id> <dir>` (kitty's syntax).
    pub terminal: String,
}

impl Default for OpencodeConfig {
    fn default() -> Self {
        Self {
            submit: false,
            terminal: "kitty".to_string(),
        }
    }
}

impl OpencodeConfig {
    fn validate(&self) -> Result<()> {
        if self.terminal.trim().is_empty() {
            bail!("opencode.terminal must not be empty");
        }
        Ok(())
    }
}

/// The toolbar beside the selection.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolbarConfig {
    /// Button size in (logical) pixels; the whole toolbar and its icons scale with it.
    pub button_size: f64,
}

impl Default for ToolbarConfig {
    fn default() -> Self {
        Self { button_size: 44.0 }
    }
}

impl ToolbarConfig {
    fn validate(&self) -> Result<()> {
        if !(24.0..=96.0).contains(&self.button_size) {
            bail!("toolbar.button_size must be between 24 and 96");
        }
        Ok(())
    }
}

/// Where Ctrl+S (and `tyshot full --save`) writes screenshots.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SaveConfig {
    /// Supports a leading `~/`. Default: XDG pictures dir + `/tyshot`.
    pub output_dir: Option<PathBuf>,
    /// File names are `<base_name>-<YYYY-MM-DD_HH-MM-SS>.png`.
    pub base_name: String,
}

impl Default for SaveConfig {
    fn default() -> Self {
        Self {
            output_dir: None,
            base_name: "tyshot".to_string(),
        }
    }
}

impl SaveConfig {
    fn validate(&self) -> Result<()> {
        if self.base_name.trim().is_empty() || self.base_name.contains('/') {
            bail!("save.base_name must be non-empty and contain no '/'");
        }
        Ok(())
    }

    pub fn resolved_output_dir(&self) -> Result<PathBuf> {
        let home = dirs::home_dir().context("unable to resolve home directory")?;
        Ok(match &self.output_dir {
            Some(dir) => expand_home(dir, &home),
            None => dirs::picture_dir()
                .unwrap_or_else(|| home.join("Pictures"))
                .join("tyshot"),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationsConfig {
    /// Show a desktop notification (via `notify-send`) after copying or saving.
    pub enabled: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Look of the selection overlay.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OverlayConfig {
    /// Selection outline and handle color, `#rrggbb`.
    pub color: String,
    /// Opacity of the shade over everything outside the selection, 0–1.
    pub dim: f64,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            color: "#e5484d".to_string(),
            dim: 0.5,
        }
    }
}

impl OverlayConfig {
    /// `color` as 0.0..=1.0 RGB components.
    pub fn rgb(&self) -> Option<(f64, f64, f64)> {
        parse_hex_color(&self.color)
    }

    fn validate(&self) -> Result<()> {
        if self.rgb().is_none() {
            bail!("overlay.color must look like #rrggbb");
        }
        if !(0.0..=1.0).contains(&self.dim) {
            bail!("overlay.dim must be between 0 and 1");
        }
        Ok(())
    }
}

/// Annotations drawn with the overlay toolbar: boxes, arrows, pen, highlighter, text,
/// numbered steps and blur.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnnotationConfig {
    /// Box, arrow, pen, text and step-marker color, `#rrggbb`.
    pub color: String,
    /// Box, arrow and pen thickness in (logical) pixels.
    pub line_width: f64,
    /// Highlighter color, `#rrggbb`. It is multiplied into the image like a real marker.
    pub highlight_color: String,
    /// Text size in (logical) pixels; numbered step markers scale with it.
    pub font_size: f64,
    /// Blur strength in (logical) pixels. Content is pixelated into blocks 3x this size and
    /// then blurred by it, so detail smaller than the blocks is gone for good.
    pub blur_radius: u32,
}

impl Default for AnnotationConfig {
    fn default() -> Self {
        Self {
            color: "#e5484d".to_string(),
            line_width: 3.0,
            highlight_color: "#ffd60a".to_string(),
            font_size: 20.0,
            blur_radius: 10,
        }
    }
}

impl AnnotationConfig {
    /// `color` as 0.0..=1.0 RGB components.
    pub fn rgb(&self) -> Option<(f64, f64, f64)> {
        parse_hex_color(&self.color)
    }

    /// `highlight_color` as 0.0..=1.0 RGB components.
    pub fn highlight_rgb(&self) -> Option<(f64, f64, f64)> {
        parse_hex_color(&self.highlight_color)
    }

    fn validate(&self) -> Result<()> {
        if self.rgb().is_none() {
            bail!("annotation.color must look like #rrggbb");
        }
        if self.highlight_rgb().is_none() {
            bail!("annotation.highlight_color must look like #rrggbb");
        }
        if !(1.0..=20.0).contains(&self.line_width) {
            bail!("annotation.line_width must be between 1 and 20");
        }
        if !(8.0..=96.0).contains(&self.font_size) {
            bail!("annotation.font_size must be between 8 and 96");
        }
        if !(1..=100).contains(&self.blur_radius) {
            bail!("annotation.blur_radius must be between 1 and 100");
        }
        Ok(())
    }
}

/// `#rrggbb` as 0.0..=1.0 RGB components.
fn parse_hex_color(color: &str) -> Option<(f64, f64, f64)> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&hex[range], 16)
            .ok()
            .map(|value| f64::from(value) / 255.0)
    };
    Some((channel(0..2)?, channel(2..4)?, channel(4..6)?))
}

/// A parsed config and the file it came from (`None` when defaults are in use).
pub type LoadedConfig = ty_common::config::Loaded<AppConfig>;

impl AppConfig {
    /// Load `config_path_override`, or `./tyshot.yaml`, or `~/.config/tyshot/config.yaml`, or
    /// fall back to the defaults when none exists.
    pub fn load(config_path_override: Option<PathBuf>) -> Result<LoadedConfig> {
        ty_common::config::load("tyshot", config_path_override)
    }

    /// Parse YAML text. An empty file is a valid "all defaults" config.
    pub fn parse(raw: &str) -> Result<Self> {
        ty_common::config::parse(raw)
    }

    pub fn validate(&self) -> Result<()> {
        self.save.validate()?;
        self.overlay.validate()?;
        self.annotation.validate()?;
        self.toolbar.validate()?;
        self.opencode.validate()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;
    use std::path::Path;

    #[test]
    fn validate_accepts_empty_config() {
        let cfg = AppConfig::parse("").expect("empty config should parse");
        cfg.validate().expect("empty config should be valid");
        assert_eq!(cfg.save.base_name, "tyshot");
        assert!(cfg.notifications.enabled);
    }

    #[test]
    fn parses_full_config() {
        let cfg = AppConfig::parse(
            "save:\n  output_dir: ~/shots\n  base_name: shot\nnotifications:\n  enabled: false\noverlay:\n  color: '#00ff80'\n  dim: 0.3\nannotation:\n  color: '#ffffff'\n  line_width: 5\n  highlight_color: '#00ff00'\n  font_size: 30\n  blur_radius: 20\ntoolbar:\n  button_size: 56\nopencode:\n  submit: true\n  terminal: /usr/bin/kitty\n",
        )
        .expect("config should parse");
        cfg.validate().expect("config should be valid");
        assert_eq!(cfg.save.base_name, "shot");
        assert!(!cfg.notifications.enabled);
        assert_eq!(cfg.overlay.rgb(), Some((0.0, 1.0, 128.0 / 255.0)));
        assert_eq!(cfg.overlay.dim, 0.3);
        assert_eq!(cfg.annotation.rgb(), Some((1.0, 1.0, 1.0)));
        assert_eq!(cfg.annotation.line_width, 5.0);
        assert_eq!(cfg.annotation.highlight_rgb(), Some((0.0, 1.0, 0.0)));
        assert_eq!(cfg.annotation.font_size, 30.0);
        assert_eq!(cfg.annotation.blur_radius, 20);
        assert_eq!(cfg.toolbar.button_size, 56.0);
        assert!(cfg.opencode.submit);
        assert_eq!(cfg.opencode.terminal, "/usr/bin/kitty");
    }

    #[test]
    fn rejects_bad_values() {
        for raw in [
            "overlay:\n  color: red\n",
            "overlay:\n  dim: 2\n",
            "save:\n  base_name: a/b\n",
            "annotation:\n  line_width: 0\n",
            "annotation:\n  blur_radius: 0\n",
            "annotation:\n  font_size: 4\n",
            "annotation:\n  highlight_color: yellow\n",
            "toolbar:\n  button_size: 8\n",
            "opencode:\n  terminal: ''\n",
        ] {
            let cfg = AppConfig::parse(raw).expect("config should parse");
            assert!(cfg.validate().is_err(), "{raw:?} should be invalid");
        }
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(AppConfig::parse("recording:\n  framerate: 30\n").is_err());
    }

    #[test]
    fn resolves_output_dir_under_home() {
        let cfg = AppConfig::parse("save:\n  output_dir: ~/shots\n").expect("config should parse");
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(cfg.save.resolved_output_dir().unwrap(), home.join("shots"));
        let cfg = AppConfig::parse("save:\n  output_dir: /abs\n").expect("config should parse");
        assert_eq!(cfg.save.resolved_output_dir().unwrap(), Path::new("/abs"));
    }
}
