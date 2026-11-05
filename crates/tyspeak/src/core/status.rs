use serde::{Deserialize, Serialize};
use ty_common::status::Report;

use crate::speech::audio::level_meter;
use crate::speech::models::human_size;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Daemon not reachable (only produced by clients).
    Offline,
    #[default]
    Idle,
    Recording,
    /// Recording finished; turning it into text.
    Transcribing,
}

/// Where the selected model is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    /// Not downloaded.
    #[default]
    Missing,
    Downloading,
    Loading,
    Ready,
    /// Download or load failed.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadStatus {
    pub model: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
}

impl DownloadStatus {
    pub fn fraction(&self) -> f32 {
        match self.total_bytes {
            0 => 0.0,
            total => (self.done_bytes as f64 / total as f64).clamp(0.0, 1.0) as f32,
        }
    }

    /// `45% (330 MB / 731 MB)`.
    pub fn describe(&self) -> String {
        format!(
            "{:.0}% ({} / {})",
            self.fraction() * 100.0,
            human_size(self.done_bytes),
            human_size(self.total_bytes)
        )
    }
}

/// Snapshot of the daemon, served over D-Bus.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Status {
    pub state: State,
    /// Selected model id.
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_state: ModelState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<DownloadStatus>,
    #[serde(default)]
    pub elapsed_secs: u64,
    /// Microphone peak level, 0.0..=1.0, while recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mic_level: Option<f32>,
}

impl Status {
    pub fn offline() -> Self {
        Self {
            state: State::Offline,
            ..Self::default()
        }
    }
}

impl Report for Status {
    /// One line for terminals and simple bars.
    fn format_plain(&self) -> String {
        let mut parts = Vec::new();
        match self.state {
            State::Offline => return "offline".to_string(),
            State::Idle => parts.push("idle".to_string()),
            State::Transcribing => parts.push("transcribing".to_string()),
            State::Recording => {
                let mut part = format!("recording {}", format_elapsed(self.elapsed_secs));
                if let Some(level) = self.mic_level {
                    part.push_str(&format!(" {}", level_meter(level)));
                }
                parts.push(part);
            }
        }
        match self.model_state {
            ModelState::Ready => parts.push(self.model.clone()),
            ModelState::Loading => parts.push(format!("loading {}", self.model)),
            ModelState::Missing => parts.push(format!("{} not downloaded", self.model)),
            ModelState::Failed => parts.push(format!("{} failed", self.model)),
            ModelState::Downloading => {}
        }
        if let Some(download) = &self.download {
            parts.push(format!(
                "downloading {} {}",
                download.model,
                download.describe()
            ));
        }
        parts.join(" | ")
    }

    /// Polybar-formatted line; empty when there is nothing to show (hides the module).
    fn format_polybar(&self) -> String {
        const RED: &str = "%{F#e5484d}";
        const YELLOW: &str = "%{F#f5c542}";
        const RESET: &str = "%{F-}";
        let mut parts = Vec::new();
        match self.state {
            State::Offline | State::Idle => {}
            State::Recording => {
                let mut part = format!("{RED}●{RESET} {}", format_elapsed(self.elapsed_secs));
                if let Some(level) = self.mic_level {
                    part.push_str(&format!(" {}", level_meter(level)));
                }
                parts.push(part);
            }
            State::Transcribing => parts.push(format!("{YELLOW}●{RESET} transcribing")),
        }
        if let Some(download) = &self.download {
            parts.push(format!(
                "{YELLOW}⤓{RESET} {:.0}%",
                download.fraction() * 100.0
            ));
        }
        parts.join("  ")
    }
}

pub fn format_elapsed(secs: u64) -> String {
    let (hours, minutes, seconds) = (secs / 3600, secs / 60 % 60, secs % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::{format_elapsed, DownloadStatus, ModelState, Report, State, Status};

    #[test]
    fn formats_elapsed() {
        assert_eq!(format_elapsed(5), "00:05");
        assert_eq!(format_elapsed(125), "02:05");
        assert_eq!(format_elapsed(3725), "1:02:05");
    }

    #[test]
    fn polybar_hides_when_idle() {
        assert_eq!(Status::default().format_polybar(), "");
    }

    #[test]
    fn formats_recording_with_download() {
        let status = Status {
            state: State::Recording,
            model: "parakeet-v2".into(),
            model_state: ModelState::Ready,
            elapsed_secs: 61,
            mic_level: Some(1.0),
            download: Some(DownloadStatus {
                model: "parakeet-v3".into(),
                done_bytes: 250_000_000,
                total_bytes: 1_000_000_000,
            }),
        };
        assert_eq!(
            status.format_plain(),
            "recording 01:01 █ | parakeet-v2 | downloading parakeet-v3 25% (250 MB / 1.0 GB)"
        );
        assert_eq!(
            status.format_polybar(),
            "%{F#e5484d}●%{F-} 01:01 █  %{F#f5c542}⤓%{F-} 25%"
        );
    }

    #[test]
    fn status_round_trips_as_json() {
        let status = Status {
            state: State::Transcribing,
            model: "sensevoice-small".into(),
            model_state: ModelState::Loading,
            ..Status::default()
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(serde_json::from_str::<Status>(&json).unwrap(), status);
    }
}
