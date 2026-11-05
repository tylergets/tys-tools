use std::path::PathBuf;

use anyhow::{bail, Result};
use serde::Deserialize;
use ty_common::config::{self, Loaded};

pub const APP_NAME: &str = "tyci";

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Seconds between refreshes.
    pub poll_seconds: u64,
    pub repos: ReposConfig,
    pub pull_requests: PullRequestsConfig,
    pub runs: RunsConfig,
    pub notifications: NotificationsConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            poll_seconds: 60,
            repos: ReposConfig::default(),
            pull_requests: PullRequestsConfig::default(),
            runs: RunsConfig::default(),
            notifications: NotificationsConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load(path: Option<PathBuf>) -> Result<Loaded<Self>> {
        let loaded: Loaded<Self> = config::load(APP_NAME, path)?;
        loaded.config.validate()?;
        Ok(loaded)
    }

    pub fn validate(&self) -> Result<()> {
        if self.poll_seconds < 10 {
            bail!("poll_seconds must be at least 10 (GitHub rate limits)");
        }
        if self.pull_requests.limit > 100 {
            bail!("pull_requests.limit must be at most 100");
        }
        for pattern in self.repos.include.iter().chain(&self.repos.exclude) {
            if pattern.trim().is_empty() {
                bail!("repos.include / repos.exclude entries must not be empty");
            }
        }
        Ok(())
    }
}

/// Which repositories are shown. `include` is a whitelist (empty means every repo),
/// `exclude` a blacklist that always wins. Entries are `owner/name`, `owner` (all of the
/// owner's repos) or globs with `*`, matched case-insensitively.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReposConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// How many of your most recently pushed repos (and of each whitelisted owner's) are
    /// checked for CI runs. 0 checks only exact `include` entries and repos with open PRs.
    pub discover: usize,
}

impl Default for ReposConfig {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
            discover: 15,
        }
    }
}

impl ReposConfig {
    /// Whether `repo` (`owner/name`) passes the whitelist and blacklist.
    pub fn allows(&self, repo: &str) -> bool {
        let included =
            self.include.is_empty() || self.include.iter().any(|p| pattern_matches(p, repo));
        included && !self.exclude.iter().any(|p| pattern_matches(p, repo))
    }

    /// `include` entries naming one repo exactly; these are always checked for runs.
    pub fn exact_includes(&self) -> impl Iterator<Item = &str> {
        self.include
            .iter()
            .map(|p| p.trim())
            .filter(|p| p.contains('/') && !p.contains('*'))
    }

    /// Owners whose repos the whitelist covers (`owner` or `owner/<glob>`), for discovery.
    pub fn included_owners(&self) -> Vec<&str> {
        let mut owners: Vec<&str> = self
            .include
            .iter()
            .map(|p| p.trim())
            .filter_map(|p| match p.split_once('/') {
                None => Some(p),
                Some((owner, name)) if name.contains('*') => Some(owner),
                Some(_) => None,
            })
            .filter(|owner| !owner.is_empty() && !owner.contains('*'))
            .collect();
        owners.dedup();
        owners
    }
}

/// `pattern` against `owner/name`. A pattern without `/` matches the owner.
pub fn pattern_matches(pattern: &str, repo: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let repo = repo.to_ascii_lowercase();
    if pattern.contains('/') {
        glob(&pattern, &repo)
    } else {
        let owner = repo.split('/').next().unwrap_or_default();
        glob(&pattern, owner)
    }
}

/// `*` matches any run of characters (including `/`).
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PullRequestsConfig {
    /// GitHub search qualifiers, e.g. `involves:@me`, `author:@me`, `review-requested:@me`,
    /// `user:my-org`. Open PRs only.
    pub query: String,
    /// Only PRs opened in the last this many days (0 for no limit).
    pub max_age_days: u32,
    /// How many PRs to show.
    pub limit: usize,
    /// Show draft PRs.
    pub drafts: bool,
}

impl Default for PullRequestsConfig {
    fn default() -> Self {
        Self {
            query: "involves:@me".to_string(),
            max_age_days: 14,
            limit: 10,
            drafts: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunsConfig {
    /// Also show runs that finished in the last this many minutes (0 to hide them).
    pub recent_minutes: u32,
    /// At most this many finished runs are shown.
    pub finished_limit: usize,
    /// Runs fetched per repo on each refresh.
    pub per_repo: usize,
}

impl Default for RunsConfig {
    fn default() -> Self {
        Self {
            recent_minutes: 60,
            finished_limit: 8,
            per_repo: 10,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationsConfig {
    /// Notify when a run tyci saw running finishes.
    pub on_finish: bool,
    /// Only notify about failed runs.
    pub only_failures: bool,
    /// Notify when a new PR shows up.
    pub on_new_pr: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            on_finish: true,
            only_failures: false,
            on_new_pr: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repos(include: &[&str], exclude: &[&str]) -> ReposConfig {
        ReposConfig {
            include: include.iter().map(|s| s.to_string()).collect(),
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
            discover: 0,
        }
    }

    #[test]
    fn globs() {
        assert!(glob("a/*", "a/b"));
        assert!(glob("*/tools", "me/tools"));
        assert!(glob("me/ty*s", "me/tys-tools"));
        assert!(!glob("me/ty*x", "me/tys-tools"));
        assert!(glob("*", "anything"));
        assert!(!glob("a/b", "a/bc"));
    }

    #[test]
    fn empty_include_allows_everything_but_excludes() {
        let r = repos(&[], &["me/secret", "bots"]);
        assert!(r.allows("me/tools"));
        assert!(!r.allows("Me/Secret"));
        assert!(!r.allows("bots/anything"));
    }

    #[test]
    fn include_is_a_whitelist_and_exclude_wins() {
        let r = repos(&["me", "org/api-*"], &["me/old"]);
        assert!(r.allows("me/tools"));
        assert!(r.allows("org/api-server"));
        assert!(!r.allows("org/web"));
        assert!(!r.allows("me/old"));
    }

    #[test]
    fn include_helpers() {
        let r = repos(&["me", "org/api-*", "x/y", "*/z"], &[]);
        assert_eq!(r.exact_includes().collect::<Vec<_>>(), ["x/y"]);
        assert_eq!(r.included_owners(), ["me", "org"]);
    }

    #[test]
    fn parses_yaml() {
        let config: AppConfig = ty_common::config::parse(
            "poll_seconds: 30\nrepos:\n  include: [me]\n  exclude: [me/old]\n",
        )
        .unwrap();
        assert_eq!(config.poll_seconds, 30);
        assert_eq!(config.repos.discover, 15);
        assert!(config.validate().is_ok());
        assert!(ty_common::config::parse::<AppConfig>("bogus: 1").is_err());
    }
}
