//! The ty* icon set: one outline glyph per tool (24px grid, 2px round strokes), drawn in
//! `currentColor` so the same SVGs work on the web. For the tray they are recolored per state.
//! The source files live in `crates/ty-common/icons/`.

use crate::tray::{BundledIcon, Icons};

/// Neutral tray color.
pub const IDLE_COLOR: &str = "#d3d2d1";
/// Needs attention, e.g. recording.
pub const ATTENTION_COLOR: &str = "#e5484d";
/// Working in the background.
pub const BUSY_COLOR: &str = "#f5c542";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Board,
    Key,
    Capture,
    Shot,
    Speak,
    Light,
    Gesture,
}

impl Tool {
    pub const ALL: [Tool; 7] = [
        Tool::Board,
        Tool::Key,
        Tool::Capture,
        Tool::Shot,
        Tool::Speak,
        Tool::Light,
        Tool::Gesture,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Board => "tyboard",
            Tool::Key => "tykey",
            Tool::Capture => "tycapture",
            Tool::Shot => "tyshot",
            Tool::Speak => "tyspeak",
            Tool::Light => "tylight",
            Tool::Gesture => "tygesture",
        }
    }

    /// The `currentColor` SVG source.
    pub fn svg(self) -> &'static str {
        match self {
            Tool::Board => include_str!("../icons/tyboard.svg"),
            Tool::Key => include_str!("../icons/tykey.svg"),
            Tool::Capture => include_str!("../icons/tycapture.svg"),
            Tool::Shot => include_str!("../icons/tyshot.svg"),
            Tool::Speak => include_str!("../icons/tyspeak.svg"),
            Tool::Light => include_str!("../icons/tylight.svg"),
            Tool::Gesture => include_str!("../icons/tygesture.svg"),
        }
    }

    /// Freedesktop icon used if the bundled ones can't be installed.
    fn fallback(self) -> &'static str {
        match self {
            Tool::Board => "edit-paste",
            Tool::Key => "input-keyboard",
            Tool::Capture => "media-record",
            Tool::Shot => "camera-photo",
            Tool::Speak => "audio-input-microphone",
            Tool::Light => "weather-clear-night",
            Tool::Gesture => "input-touchpad",
        }
    }

    /// The SVG drawn in `color`.
    pub fn svg_colored(self, color: &str) -> String {
        self.svg().replace("currentColor", color)
    }

    /// Idle, attention and busy tray icons, named `<tool>`, `<tool>-attention`, `<tool>-busy`.
    pub fn tray_icons(self) -> Icons {
        let icon = |suffix: &str, color: &str| BundledIcon {
            name: format!("{}{suffix}", self.name()),
            svg: self.svg_colored(color),
        };
        Icons {
            idle: icon("", IDLE_COLOR),
            attention: Some(icon("-attention", ATTENTION_COLOR)),
            busy: Some(icon("-busy", BUSY_COLOR)),
            fallback: self.fallback().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_share_the_style() {
        for tool in Tool::ALL {
            let svg = tool.svg();
            assert!(svg.contains(r#"viewBox="0 0 24 24""#), "{}", tool.name());
            assert!(svg.contains(r#"stroke-width="2""#), "{}", tool.name());
            assert!(svg.trim_end().ends_with("</svg>"), "{}", tool.name());
        }
    }

    #[test]
    fn tray_icons_are_recolored() {
        let icons = Tool::Speak.tray_icons();
        assert!(icons.idle.svg.contains(IDLE_COLOR));
        assert!(!icons.idle.svg.contains("currentColor"));
        assert_eq!(icons.attention.unwrap().name, "tyspeak-attention");
    }
}
