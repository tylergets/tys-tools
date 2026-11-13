use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local, Utc};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub directory: Option<PathBuf>,
    pub default_author: Option<String>,
    pub show_author: bool,
    pub author_prefix: String,
    pub day_boundary: DayBoundary,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            directory: None,
            default_author: None,
            show_author: true,
            author_prefix: "— ".into(),
            day_boundary: DayBoundary::Local,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayBoundary {
    Local,
    Utc,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self
            .directory
            .as_ref()
            .is_some_and(|p| p.as_os_str().is_empty())
        {
            bail!("directory cannot be empty");
        }
        if self
            .default_author
            .as_ref()
            .is_some_and(|s| s.trim().is_empty())
        {
            bail!("default_author cannot be blank; use null for no default");
        }
        Ok(())
    }

    pub fn directory(&self, override_path: Option<PathBuf>) -> Result<PathBuf> {
        let path = override_path
            .or_else(|| self.directory.clone())
            .or_else(|| dirs::data_dir().map(|p| p.join("tyquote/quotes")))
            .context("cannot locate data directory; set directory or pass --directory")?;
        if path.as_os_str().is_empty() {
            bail!("directory cannot be empty");
        }
        Ok(ty_common::config::expand_user(&path))
    }

    pub fn date(&self, now: DateTime<Utc>) -> String {
        match self.day_boundary {
            DayBoundary::Local => now.with_timezone(&Local).format("%Y-%m-%d").to_string(),
            DayBoundary::Utc => now.format("%Y-%m-%d").to_string(),
        }
    }

    pub fn display(&self, text: &str, author: Option<&str>) -> String {
        match author.filter(|_| self.show_author) {
            Some(author) => format!("{text}\n{}{author}", self.author_prefix),
            None => text.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_output_and_reject_invalid_settings() {
        let config: Config = ty_common::config::parse("").unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.display("Quote", Some("Author")), "Quote\n— Author");
        for yaml in ["directory: ''", "default_author: ' '"] {
            assert!(ty_common::config::parse::<Config>(yaml)
                .unwrap()
                .validate()
                .is_err());
        }
        for yaml in [
            "show_author: nope",
            "day_boundary: tomorrow",
            "unknown: true",
        ] {
            assert!(ty_common::config::parse::<Config>(yaml).is_err());
        }
    }

    #[test]
    fn configured_directory_can_be_overridden() {
        let config: Config = ty_common::config::parse("directory: ~/Quotes").unwrap();
        assert_eq!(
            config.directory(None).unwrap(),
            dirs::home_dir().unwrap().join("Quotes")
        );
        assert_eq!(
            config.directory(Some("other".into())).unwrap(),
            PathBuf::from("other")
        );
    }

    #[test]
    fn output_and_utc_boundary_are_configurable() {
        let mut config: Config =
            ty_common::config::parse("author_prefix: 'by '\nday_boundary: utc").unwrap();
        assert_eq!(config.display("Quote", Some("Author")), "Quote\nby Author");
        config.show_author = false;
        assert_eq!(config.display("Quote", Some("Author")), "Quote");
        let before: DateTime<Utc> = "2026-11-12T23:59:59Z".parse().unwrap();
        assert_eq!(config.date(before), "2026-11-12");
        assert_eq!(
            config.date(before + chrono::Duration::seconds(1)),
            "2026-11-13"
        );
    }
}
