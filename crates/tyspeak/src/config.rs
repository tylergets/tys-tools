use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use ty_common::config::{self, Loaded};

use crate::speech::models::{self, DEFAULT_MODEL};

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Model used until one is picked from the tray (the tray choice is remembered).
    pub model: String,
    /// Where models are stored. Default: `$XDG_DATA_HOME/tyspeak/models`.
    pub model_dir: Option<PathBuf>,
    /// Download the selected model on startup if it is missing.
    pub auto_download: bool,
    /// Language hint (ISO code such as `en` or `de`) for models that take one; unset lets the
    /// model decide.
    pub language: Option<String>,
    /// CPU threads for inference; 0 picks automatically.
    pub threads: u32,
    pub audio: AudioConfig,
    pub output: OutputConfig,
    pub notifications: NotificationsConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            model_dir: None,
            auto_download: true,
            language: None,
            threads: 0,
            audio: AudioConfig::default(),
            output: OutputConfig::default(),
            notifications: NotificationsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    /// PulseAudio/PipeWire source name (`pactl list short sources`), or `default`.
    pub source: String,
    /// Recording stops by itself after this many seconds.
    pub max_duration: u32,
    /// Keep recording this long after stop is requested, so the last word isn't clipped. A
    /// start within this window continues the recording (key autorepeat on hold-to-talk).
    pub tail_ms: u32,
    /// Play a short chime when recording starts.
    pub start_sound: bool,
    /// Play a short chime when recording stops.
    pub stop_sound: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            source: "default".to_string(),
            max_duration: 300,
            tail_ms: 250,
            start_sound: true,
            stop_sound: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// Type the text into the focused window (xdotool).
    #[default]
    Type,
    /// Put the text on the clipboard.
    Clipboard,
    /// Both of the above.
    Both,
    /// Only emit the D-Bus `Transcribed` signal.
    None,
}

impl OutputMode {
    pub fn types(self) -> bool {
        matches!(self, Self::Type | Self::Both)
    }

    pub fn copies(self) -> bool {
        matches!(self, Self::Clipboard | Self::Both)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputConfig {
    pub mode: OutputMode,
    /// Append a space so consecutive dictations don't run together.
    pub trailing_space: bool,
    /// Delay between typed characters, in milliseconds.
    pub type_delay_ms: u32,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            mode: OutputMode::Type,
            trailing_space: true,
            type_delay_ms: 12,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationsConfig {
    /// Show desktop notifications for downloads and errors.
    pub enabled: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl AppConfig {
    /// Load `--config`, or the discovered config, or the defaults if there is none.
    pub fn load(config_path_override: Option<PathBuf>) -> Result<Loaded<Self>> {
        config::load("tyspeak", config_path_override)
    }

    pub fn parse(raw: &str) -> Result<Self> {
        config::parse(raw)
    }

    pub fn validate(&self) -> Result<()> {
        if models::find(&self.model).is_none() {
            let known: Vec<&str> = models::MODELS.iter().map(|model| model.id).collect();
            bail!(
                "unknown model '{}' (known: {})",
                self.model,
                known.join(", ")
            );
        }
        if self
            .language
            .as_deref()
            .is_some_and(|language| language.trim().is_empty())
        {
            bail!("language cannot be empty (leave it unset to let the model decide)");
        }
        if self.audio.source.trim().is_empty() {
            bail!("audio.source cannot be empty");
        }
        if !(1..=3600).contains(&self.audio.max_duration) {
            bail!("audio.max_duration must be between 1 and 3600 seconds");
        }
        if self.audio.tail_ms > 2000 {
            bail!("audio.tail_ms must be at most 2000");
        }
        if self.output.type_delay_ms > 1000 {
            bail!("output.type_delay_ms must be at most 1000");
        }
        Ok(())
    }

    pub fn resolved_model_dir(&self) -> Result<PathBuf> {
        let home = dirs::home_dir().context("unable to resolve home directory")?;
        Ok(match &self.model_dir {
            Some(dir) => config::expand_home(dir, &home),
            None => dirs::data_dir()
                .unwrap_or_else(|| home.join(".local/share"))
                .join("tyspeak")
                .join("models"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, OutputMode};

    #[test]
    fn validate_accepts_empty_config() {
        let cfg = AppConfig::parse("").expect("empty config should parse");
        cfg.validate().expect("empty config should be valid");
        assert_eq!(cfg.model, "parakeet-unified-en");
        assert_eq!(cfg.output.mode, OutputMode::Type);
        assert_eq!(cfg.audio.source, "default");
        assert!(cfg.audio.start_sound && cfg.audio.stop_sound);
    }

    #[test]
    fn parses_full_config() {
        let cfg = AppConfig::parse(
            "model: whisper-large-v3-turbo\nmodel_dir: ~/models\nauto_download: false\nlanguage: de\nthreads: 4\naudio:\n  source: alsa_input.usb\n  max_duration: 60\n  tail_ms: 100\n  start_sound: false\n  stop_sound: false\noutput:\n  mode: both\n  trailing_space: false\n  type_delay_ms: 5\nnotifications:\n  enabled: false\n",
        )
        .expect("config should parse");
        cfg.validate().expect("config should be valid");
        assert_eq!(cfg.model, "whisper-large-v3-turbo");
        assert!(!cfg.auto_download);
        assert_eq!(cfg.language.as_deref(), Some("de"));
        assert_eq!(cfg.audio.max_duration, 60);
        assert!(!cfg.audio.start_sound && !cfg.audio.stop_sound);
        assert_eq!(cfg.output.mode, OutputMode::Both);
        assert!(cfg.output.mode.types() && cfg.output.mode.copies());
        assert!(!cfg.notifications.enabled);
    }

    #[test]
    fn rejects_unknown_model_and_fields() {
        let cfg = AppConfig::parse("model: nope\n").expect("config should parse");
        assert!(cfg.validate().is_err());
        assert!(AppConfig::parse("recording:\n  framerate: 30\n").is_err());
        assert!(AppConfig::parse("output:\n  mode: shout\n").is_err());
    }

    #[test]
    fn rejects_out_of_range_values() {
        for raw in [
            "audio:\n  max_duration: 0\n",
            "audio:\n  tail_ms: 5000\n",
            "output:\n  type_delay_ms: 5000\n",
            "language: ''\n",
        ] {
            let cfg = AppConfig::parse(raw).expect("config should parse");
            assert!(cfg.validate().is_err(), "{raw}");
        }
    }
}
