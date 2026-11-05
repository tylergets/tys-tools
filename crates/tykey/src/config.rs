use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

pub const APP_NAME: &str = "tykey";

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub expansions: Vec<ExpansionRule>,
    #[serde(default)]
    pub snippets: Vec<MenuSnippet>,
    #[serde(default)]
    pub globals: HashMap<String, String>,
    #[serde(default)]
    pub notifications: NotificationConfig,
    #[serde(default)]
    pub match_behavior: MatchBehavior,
    pub boundary_chars: Option<String>,
    #[serde(default)]
    pub watch: bool,
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub config: AppConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExpansionRule {
    pub trigger: String,
    pub expansion: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MenuSnippet {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NotificationConfig {
    #[serde(default)]
    pub on_expansion: bool,
    #[serde(default)]
    pub on_snippet_copy: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchBehavior {
    #[default]
    Immediate,
    Boundary,
}

impl AppConfig {
    pub fn load(config_path_override: Option<PathBuf>) -> Result<LoadedConfig> {
        let loaded = ty_common::config::load_required::<AppConfig>(APP_NAME, config_path_override)?;
        let path = loaded.path.context("config loaded without a source path")?;
        Ok(LoadedConfig {
            path,
            config: loaded.config,
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.expansions.is_empty() {
            bail!("config must include at least one expansion");
        }

        let mut seen = HashSet::new();
        for rule in &self.expansions {
            if rule.trigger.is_empty() {
                bail!("trigger cannot be empty");
            }
            if !seen.insert(rule.trigger.clone()) {
                bail!("duplicate trigger found: {}", rule.trigger);
            }
        }

        let mut seen_titles = HashSet::new();
        for snippet in &self.snippets {
            if snippet.title.trim().is_empty() {
                bail!("snippet title cannot be empty");
            }
            if snippet.content.is_empty() {
                bail!("snippet content cannot be empty");
            }
            if !seen_titles.insert(snippet.title.clone()) {
                bail!("duplicate snippet title found: {}", snippet.title);
            }
        }

        let mut seen_global_names = HashSet::new();
        for name in self.globals.keys() {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                bail!("global macro name cannot be empty");
            }
            if trimmed.contains('{') || trimmed.contains('}') || trimmed.contains(':') {
                bail!("global macro name contains unsupported characters: {trimmed}");
            }
            if !seen_global_names.insert(trimmed.to_ascii_uppercase()) {
                bail!("duplicate global macro name found (case-insensitive): {trimmed}");
            }
        }

        Ok(())
    }

    pub fn boundary_chars(&self) -> &str {
        self.boundary_chars
            .as_deref()
            .unwrap_or(" \t\n.,;:!?)]}>'\"")
    }
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, ExpansionRule, MatchBehavior, MenuSnippet, NotificationConfig};
    use std::collections::HashMap;

    fn sample_rule(trigger: &str, expansion: &str) -> ExpansionRule {
        ExpansionRule {
            trigger: trigger.to_string(),
            expansion: expansion.to_string(),
        }
    }

    fn sample_snippet(title: &str, content: &str) -> MenuSnippet {
        MenuSnippet {
            title: title.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn validate_rejects_empty_expansions() {
        let cfg = AppConfig {
            expansions: vec![],
            snippets: vec![],
            globals: HashMap::new(),
            notifications: NotificationConfig::default(),
            match_behavior: MatchBehavior::Immediate,
            boundary_chars: None,
            watch: false,
        };

        let err = cfg.validate().expect_err("empty config should fail");
        assert!(err.to_string().contains("at least one expansion"));
    }

    #[test]
    fn validate_rejects_duplicate_triggers() {
        let cfg = AppConfig {
            expansions: vec![sample_rule(";a", "alpha"), sample_rule(";a", "again")],
            snippets: vec![],
            globals: HashMap::new(),
            notifications: NotificationConfig::default(),
            match_behavior: MatchBehavior::Immediate,
            boundary_chars: None,
            watch: false,
        };

        let err = cfg.validate().expect_err("duplicate trigger should fail");
        assert!(err.to_string().contains("duplicate trigger"));
    }

    #[test]
    fn boundary_chars_uses_default_when_unset() {
        let cfg = AppConfig {
            expansions: vec![sample_rule(";a", "alpha")],
            snippets: vec![],
            globals: HashMap::new(),
            notifications: NotificationConfig::default(),
            match_behavior: MatchBehavior::Immediate,
            boundary_chars: None,
            watch: false,
        };

        assert_eq!(cfg.boundary_chars(), " \t\n.,;:!?)]}>'\"");
    }

    #[test]
    fn validate_rejects_empty_snippet_title() {
        let cfg = AppConfig {
            expansions: vec![sample_rule(";a", "alpha")],
            snippets: vec![sample_snippet(" ", "hello")],
            globals: HashMap::new(),
            notifications: NotificationConfig::default(),
            match_behavior: MatchBehavior::Immediate,
            boundary_chars: None,
            watch: false,
        };

        let err = cfg.validate().expect_err("empty snippet title should fail");
        assert!(err.to_string().contains("snippet title cannot be empty"));
    }

    #[test]
    fn validate_rejects_duplicate_snippet_titles() {
        let cfg = AppConfig {
            expansions: vec![sample_rule(";a", "alpha")],
            snippets: vec![
                sample_snippet("Email", "a@example.com"),
                sample_snippet("Email", "b@example.com"),
            ],
            globals: HashMap::new(),
            notifications: NotificationConfig::default(),
            match_behavior: MatchBehavior::Immediate,
            boundary_chars: None,
            watch: false,
        };

        let err = cfg
            .validate()
            .expect_err("duplicate snippet title should fail");
        assert!(err.to_string().contains("duplicate snippet title"));
    }
}
