use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use ty_common::config::{expand_home, expand_user};
use ty_common::frame::Frame;

use crate::capture::encoder::EncoderChoice;
use crate::capture::options::{Format, Quality, RecordingOptions};
use crate::zoom::ZoomSettings;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default)]
    pub recording: RecordingConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub window: WindowConfig,
    #[serde(default)]
    pub notifications: NotificationsConfig,
    #[serde(default)]
    pub intro: IntroConfig,
    #[serde(default)]
    pub zoom: ZoomSettings,
    #[serde(default)]
    pub marker: MarkerConfig,
    #[serde(default)]
    pub frame: FrameConfig,
}

/// Decorative frame (browser, phone, app window) around the video, added when
/// post-processing. `style` is the default for the setup dialog.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FrameConfig {
    pub style: Frame,
    /// `#rrggbb` shown outside the frame's rounded corners.
    pub background: String,
    /// Size of the frame's bars and borders; 2.0 suits a HiDPI screen.
    pub scale: f64,
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self {
            style: Frame::None,
            background: "#ffffff".to_string(),
            scale: 1.0,
        }
    }
}

impl FrameConfig {
    /// `background` as 0.0..=1.0 RGB components.
    pub fn background_rgb(&self) -> Option<(f64, f64, f64)> {
        parse_hex_color(&self.background)
    }

    fn validate(&self) -> Result<()> {
        if self.background_rgb().is_none() {
            bail!("frame.background must look like #rrggbb");
        }
        if !(0.5..=4.0).contains(&self.scale) {
            bail!("frame.scale must be between 0.5 and 4.0");
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

/// Border drawn around the recorded area while recording (outside it, so it is never
/// captured).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MarkerConfig {
    pub enabled: bool,
    /// `#rrggbb`.
    pub color: String,
    /// Border thickness in pixels.
    pub width: u32,
}

impl Default for MarkerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            color: "#e5484d".to_string(),
            width: 3,
        }
    }
}

impl MarkerConfig {
    /// `color` as 0.0..=1.0 RGB components.
    pub fn rgb(&self) -> Option<(f64, f64, f64)> {
        parse_hex_color(&self.color)
    }

    fn validate(&self) -> Result<()> {
        if self.rgb().is_none() {
            bail!("marker.color must look like #rrggbb");
        }
        if !(1..=32).contains(&self.width) {
            bail!("marker.width must be between 1 and 32");
        }
        Ok(())
    }
}

/// Title card prepended to recordings. `enabled`, `title` and `logo` are defaults for the
/// setup dialog.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IntroConfig {
    pub enabled: bool,
    pub title: String,
    /// Image shown above the title. Supports a leading `~/`.
    pub logo: Option<PathBuf>,
    /// Length of the title card in seconds, including its fades.
    pub duration: f64,
    /// fontconfig pattern for the title.
    pub font: String,
}

impl Default for IntroConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            title: String::new(),
            logo: None,
            duration: 3.0,
            font: "Sans".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationsConfig {
    /// Pause dunst while recording (restored afterwards) so popups stay out of the video.
    pub pause_dunst: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self { pause_dunst: true }
    }
}

/// Recording defaults. Everything except `output_dir` and `ffmpeg` can be changed per
/// recording in the setup dialog.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecordingConfig {
    /// Directory recordings are written to. Supports a leading `~/`.
    pub output_dir: Option<PathBuf>,
    pub base_name: String,
    pub framerate: u32,
    pub draw_mouse: bool,
    pub format: Format,
    pub quality: Quality,
    pub countdown: u32,
    pub compress: bool,
    pub keep_capture: bool,
    pub ffmpeg: String,
    /// Video encoder: auto-detected hardware encoding, or forced.
    pub encoder: EncoderChoice,
    /// DRM render node for VAAPI; `None` tries each `/dev/dri/renderD*`.
    pub vaapi_device: Option<PathBuf>,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            output_dir: None,
            base_name: "tycapture".to_string(),
            framerate: 30,
            draw_mouse: true,
            format: Format::default(),
            quality: Quality::default(),
            countdown: 0,
            compress: true,
            keep_capture: true,
            ffmpeg: "ffmpeg".to_string(),
            encoder: EncoderChoice::default(),
            vaapi_device: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    pub microphone: bool,
    /// PulseAudio/PipeWire source name; `None` uses the default source.
    pub microphone_source: Option<String>,
    /// Denoise and level the microphone (high-pass, FFT denoise, speech normalizer, limiter).
    pub mic_cleanup: bool,
    pub system_audio: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowConfig {
    /// i3 workspace a window is moved to when recording a single window.
    pub workspace: String,
    /// Put the window back (workspace, tiling/floating, border) when recording ends.
    pub restore: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            workspace: "tycapture".to_string(),
            restore: true,
        }
    }
}

pub type LoadedConfig = ty_common::config::Loaded<AppConfig>;

impl AppConfig {
    /// Load `config_path_override`, the discovered config, or defaults if there is none.
    pub fn load(config_path_override: Option<PathBuf>) -> Result<LoadedConfig> {
        ty_common::config::load("tycapture", config_path_override)
    }

    /// Parse YAML text; an empty file is a valid "all defaults" config.
    pub fn parse(raw: &str) -> Result<Self> {
        ty_common::config::parse(raw)
    }

    pub fn validate(&self) -> Result<()> {
        self.recording.validate()?;
        self.window.validate()?;
        self.intro.validate()?;
        self.zoom.validate()?;
        self.marker.validate()?;
        self.frame.validate()?;
        Ok(())
    }
}

impl RecordingConfig {
    fn validate(&self) -> Result<()> {
        if !(1..=240).contains(&self.framerate) {
            bail!("recording.framerate must be between 1 and 240");
        }
        if self.ffmpeg.trim().is_empty() {
            bail!("recording.ffmpeg cannot be empty");
        }
        if self.countdown > 60 {
            bail!("recording.countdown must be at most 60 seconds");
        }
        Ok(())
    }

    pub fn resolved_output_dir(&self) -> Result<PathBuf> {
        let home = dirs::home_dir().context("unable to resolve home directory")?;
        Ok(match &self.output_dir {
            Some(dir) => expand_home(dir, &home),
            None => dirs::video_dir()
                .unwrap_or_else(|| home.join("Videos"))
                .join("tycapture"),
        })
    }
}

impl AppConfig {
    /// Initial setup-dialog choices.
    pub fn default_options(&self) -> RecordingOptions {
        RecordingOptions {
            base_name: self.recording.base_name.clone(),
            framerate: self.recording.framerate,
            draw_mouse: self.recording.draw_mouse,
            format: self.recording.format,
            quality: self.recording.quality,
            countdown: self.recording.countdown,
            microphone: self.audio.microphone.then(|| {
                self.audio
                    .microphone_source
                    .clone()
                    .unwrap_or_else(|| "default".to_string())
            }),
            mic_cleanup: self.audio.mic_cleanup,
            system_audio: self.audio.system_audio,
            compress: self.recording.compress,
            keep_capture: self.recording.keep_capture,
            window_size: None,
            intro: self.intro.enabled,
            intro_title: self.intro.title.clone(),
            intro_logo: self.intro.logo.as_deref().map(expand_user),
            zoom: self.zoom.enabled,
            frame: self.frame.style,
        }
    }
}

impl IntroConfig {
    fn validate(&self) -> Result<()> {
        if !(1.0..=10.0).contains(&self.duration) {
            bail!("intro.duration must be between 1 and 10 seconds");
        }
        if self.font.trim().is_empty() {
            bail!("intro.font cannot be empty");
        }
        Ok(())
    }
}

impl WindowConfig {
    fn validate(&self) -> Result<()> {
        if self.workspace.trim().is_empty() {
            bail!("window.workspace cannot be empty");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;
    use crate::capture::encoder::EncoderChoice;
    use crate::capture::options::{Format, Quality};
    use std::path::Path;

    #[test]
    fn validate_accepts_empty_config() {
        let cfg = AppConfig::parse("").expect("empty config should parse");
        cfg.validate().expect("empty config should be valid");
        assert_eq!(cfg.recording.framerate, 30);
        assert_eq!(cfg.window.workspace, "tycapture");
    }

    #[test]
    fn parses_full_config() {
        let cfg = AppConfig::parse(
            "recording:\n  output_dir: ~/clips\n  framerate: 60\n  draw_mouse: false\n  format: mp4\n  quality: medium\naudio:\n  microphone: true\nwindow:\n  workspace: rec\n",
        )
        .expect("config should parse");
        cfg.validate().expect("config should be valid");
        assert_eq!(cfg.recording.framerate, 60);
        assert!(!cfg.recording.draw_mouse);
        assert_eq!(cfg.window.workspace, "rec");
        let options = cfg.default_options();
        assert_eq!(options.format, Format::Mp4);
        assert_eq!(options.quality, Quality::Medium);
        assert_eq!(options.microphone.as_deref(), Some("default"));
    }

    #[test]
    fn parses_intro() {
        let cfg = AppConfig::parse(
            "intro:\n  enabled: true\n  title: Demo\n  logo: /logo.png\n  duration: 4\n",
        )
        .expect("config should parse");
        cfg.validate().expect("config should be valid");
        assert_eq!(cfg.intro.duration, 4.0);
        let options = cfg.default_options();
        assert!(options.intro);
        assert_eq!(options.intro_title, "Demo");
        assert_eq!(options.intro_logo.as_deref(), Some(Path::new("/logo.png")));
    }

    #[test]
    fn parses_frame() {
        let cfg = AppConfig::parse("frame:\n  style: browser\n  background: \"#101010\"\n")
            .expect("config should parse");
        cfg.validate().expect("frame config should be valid");
        assert_eq!(
            cfg.default_options().frame,
            ty_common::frame::Frame::Browser
        );
        assert!(AppConfig::parse("frame:\n  style: tablet\n").is_err());
        let bad = AppConfig::parse("frame:\n  background: white\n").unwrap();
        assert!(bad.validate().is_err());
    }

    #[test]
    fn rejects_long_intro() {
        let cfg = AppConfig::parse("intro:\n  duration: 30\n").expect("config should parse");
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn parses_encoder_cleanup_and_marker() {
        let cfg = AppConfig::parse(
            "recording:\n  encoder: nvenc\naudio:\n  mic_cleanup: true\nmarker:\n  color: '#00ff80'\n  width: 4\n",
        )
        .expect("config should parse");
        cfg.validate().expect("config should be valid");
        assert_eq!(cfg.recording.encoder, EncoderChoice::Nvenc);
        assert!(cfg.default_options().mic_cleanup);
        assert_eq!(cfg.marker.rgb(), Some((0.0, 1.0, 128.0 / 255.0)));
        assert!(AppConfig::parse("recording:\n  encoder: quicksync\n").is_err());
    }

    #[test]
    fn rejects_bad_marker_color() {
        let cfg = AppConfig::parse("marker:\n  color: red\n").expect("config should parse");
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(AppConfig::parse("clipboard:\n  backend: gtk\n").is_err());
    }

    #[test]
    fn rejects_zero_framerate() {
        let cfg = AppConfig::parse("recording:\n  framerate: 0\n").expect("config should parse");
        assert!(cfg.validate().is_err());
    }
}
