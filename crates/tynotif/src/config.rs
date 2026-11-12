use anyhow::{ensure, Result};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub markup: Markup,
    pub format: Option<String>,
    pub show_indicators: bool,
    pub action_name: String,
    pub mouse_left_click: Vec<MouseAction>,
    pub mouse_middle_click: Vec<MouseAction>,
    pub mouse_right_click: Vec<MouseAction>,
    pub dmenu: Vec<String>,
    pub browser: Vec<String>,
    pub shortcuts: std::collections::BTreeMap<String, String>,
    pub width: i32,
    pub margin: i32,
    pub top: i32,
    pub gap: i32,
    pub limit: usize,
    pub history_limit: usize,
    pub timeout_ms: u64,
    pub low_timeout_ms: u64,
    pub background: String,
    pub foreground: String,
    pub border: String,
    pub critical: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            markup: Markup::Full,
            format: None,
            show_indicators: true,
            action_name: "default".into(),
            mouse_left_click: vec![MouseAction::DoAction],
            mouse_middle_click: vec![MouseAction::OpenUrl],
            mouse_right_click: vec![MouseAction::Context],
            dmenu: Vec::new(),
            browser: vec!["xdg-open".into()],
            shortcuts: Default::default(),
            width: 480,
            margin: 18,
            top: 48,
            gap: 12,
            limit: 0,
            history_limit: 200,
            timeout_ms: 20000,
            low_timeout_ms: 5000,
            background: "#202432".into(),
            foreground: "#eef0f6".into(),
            border: "#596580".into(),
            critical: "#e78284".into(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.browser.is_empty() && !self.browser[0].is_empty(),
            "browser needs an executable"
        );
        ensure!(
            self.dmenu.first().is_none_or(|s| !s.is_empty()),
            "dmenu needs an executable"
        );
        for (action, chord) in &self.shortcuts {
            ensure!(
                ["close", "close-all", "history-pop", "context", "action"]
                    .contains(&action.as_str()),
                "unknown shortcut action: {action}"
            );
            ty_common::input::validate_shortcut(chord)?;
        }
        ensure!(
            (200..=1600).contains(&self.width),
            "width must be 200..1600"
        );
        ensure!(
            self.limit <= 512,
            "limit must be 0..512 (0 fills the available screen height)"
        );
        ensure!(
            self.history_limit <= 10000,
            "history_limit must be <= 10000"
        );
        ensure!(
            [self.margin, self.top, self.gap]
                .iter()
                .all(|n| (0..=1000).contains(n)),
            "offsets must be 0..1000"
        );
        for color in [
            &self.background,
            &self.foreground,
            &self.border,
            &self.critical,
        ] {
            ensure!(
                color.len() == 7
                    && color.starts_with('#')
                    && color[1..].bytes().all(|c| c.is_ascii_hexdigit()),
                "colors must be #RRGGBB"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Markup {
    Full,
    Strip,
    No,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseAction {
    None,
    DoAction,
    OpenUrl,
    CloseCurrent,
    CloseAll,
    Context,
    ContextAll,
}
