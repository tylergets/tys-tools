//! `tykey status`: what the daemon reports to bar modules.

use serde::{Deserialize, Serialize};
use ty_common::status::Report;

use crate::core::engine::Activity;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub running: bool,
    /// Expansions in the config.
    pub expansions: usize,
    /// Expansions typed since the daemon started.
    pub expanded: usize,
    /// Trigger of the most recent one.
    pub last_trigger: Option<String>,
}

impl DaemonStatus {
    pub fn of(activity: &Activity) -> Self {
        Self {
            running: true,
            expansions: activity.expansions,
            expanded: activity.expanded,
            last_trigger: activity.last_trigger.clone(),
        }
    }
}

impl Report for DaemonStatus {
    fn format_plain(&self) -> String {
        if !self.running {
            return "tykey is not running".into();
        }
        let mut line = format!(
            "{} expansions · {} expanded",
            self.expansions, self.expanded
        );
        if let Some(trigger) = &self.last_trigger {
            line.push_str(&format!(" · last {trigger}"));
        }
        line
    }

    fn format_polybar(&self) -> String {
        if self.running {
            "tykey".into()
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_for_bars() {
        let off = DaemonStatus::default();
        assert_eq!(off.format_plain(), "tykey is not running");
        assert_eq!(off.format_polybar(), "");

        let on = DaemonStatus {
            running: true,
            expansions: 7,
            expanded: 2,
            last_trigger: Some("tg;".into()),
        };
        assert_eq!(on.format_plain(), "7 expansions · 2 expanded · last tg;");
        assert_eq!(on.format_polybar(), "tykey");
    }
}
