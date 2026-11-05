//! `tygesture status`: what the daemon reports to bar modules.

use serde::{Deserialize, Serialize};
use ty_common::status::Report;

use crate::state::Status;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub paused: bool,
    /// Name of the touchpad in use, if one is connected.
    pub touchpad: Option<String>,
    /// Gestures that ran a command since the daemon started.
    pub fired: usize,
    /// The most recent of those.
    pub last_gesture: Option<String>,
    /// Gestures bound in the config.
    pub bindings: usize,
}

impl DaemonStatus {
    pub fn of(status: &Status) -> Self {
        Self {
            running: true,
            paused: status.paused,
            touchpad: status.touchpad.clone(),
            fired: status.fired,
            last_gesture: status.history.back().map(|f| f.gesture.clone()),
            bindings: status.bindings.len(),
        }
    }
}

impl Report for DaemonStatus {
    fn format_plain(&self) -> String {
        if !self.running {
            return "tygesture is not running".into();
        }
        let mut parts = vec![if self.paused { "paused" } else { "on" }.to_string()];
        parts.push(
            self.touchpad
                .clone()
                .unwrap_or_else(|| "no touchpad".into()),
        );
        parts.push(format!("{} fired", self.fired));
        if let Some(last) = &self.last_gesture {
            parts.push(format!("last {last}"));
        }
        parts.join(" · ")
    }

    fn format_polybar(&self) -> String {
        match (self.running, self.paused) {
            (false, _) => String::new(),
            (true, true) => "gestures off".into(),
            (true, false) => "gestures".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_for_bars() {
        let off = DaemonStatus::default();
        assert_eq!(off.format_plain(), "tygesture is not running");
        assert_eq!(off.format_polybar(), "");

        let on = DaemonStatus {
            running: true,
            touchpad: Some("Touchpad".into()),
            fired: 3,
            last_gesture: Some("swipe 3 left".into()),
            bindings: 4,
            ..DaemonStatus::default()
        };
        assert_eq!(
            on.format_plain(),
            "on · Touchpad · 3 fired · last swipe 3 left"
        );
        assert_eq!(on.format_polybar(), "gestures");
        let paused = DaemonStatus { paused: true, ..on };
        assert_eq!(paused.format_polybar(), "gestures off");
    }
}
