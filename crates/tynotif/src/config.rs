use anyhow::{ensure, Result};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
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
            width: 480,
            margin: 18,
            top: 48,
            gap: 12,
            limit: 4,
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
            (200..=1600).contains(&self.width),
            "width must be 200..1600"
        );
        ensure!((1..=10).contains(&self.limit), "limit must be 1..10");
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
