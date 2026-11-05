//! One refresh: open PRs, then the CI runs of every watched repo.

use std::collections::HashSet;
use std::thread;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use ty_common::icons::{ATTENTION_COLOR, BUSY_COLOR};
use ty_common::status::Report;

use crate::config::AppConfig;
use crate::github::{self, PullRequest, Run};

/// How many `gh run list` calls run at once.
const PARALLEL_REPOS: usize = 8;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub fetched_at: Option<DateTime<Utc>>,
    pub pull_requests: Vec<PullRequest>,
    /// Queued and in-progress runs, newest first.
    pub running: Vec<Run>,
    /// Runs that finished within `runs.recent_minutes`, newest first.
    pub finished: Vec<Run>,
    /// Repos checked for runs.
    pub repos: Vec<String>,
    /// gh failures; the rest of the snapshot is still filled in.
    pub errors: Vec<String>,
}

impl Snapshot {
    pub fn fetch(config: &AppConfig) -> Self {
        let now = Utc::now();
        let mut snapshot = Snapshot {
            fetched_at: Some(now),
            ..Snapshot::default()
        };

        match fetch_prs(config, now) {
            Ok(prs) => snapshot.pull_requests = prs,
            Err(err) => snapshot.errors.push(format!("{err:#}")),
        }

        snapshot.repos = watched_repos(config, &snapshot.pull_requests, &mut snapshot.errors);

        let mut runs = Vec::new();
        for chunk in snapshot.repos.chunks(PARALLEL_REPOS) {
            let results: Vec<_> = thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|repo| scope.spawn(|| github::list_runs(repo, config.runs.per_repo)))
                    .collect();
                handles.into_iter().map(|h| h.join()).collect()
            });
            for result in results {
                match result {
                    Ok(Ok(found)) => runs.extend(found),
                    Ok(Err(err)) => snapshot.errors.push(format!("{err:#}")),
                    Err(_) => snapshot.errors.push("gh run list panicked".into()),
                }
            }
        }
        let (running, finished) = classify(runs, now, config.runs.recent_minutes);
        snapshot.running = running;
        snapshot.finished = finished;
        snapshot.finished.truncate(config.runs.finished_limit);
        snapshot
    }

    pub fn failed(&self) -> impl Iterator<Item = &Run> {
        self.finished.iter().filter(|run| run.failed())
    }

    pub fn running_ids(&self) -> HashSet<u64> {
        self.running.iter().map(|run| run.id).collect()
    }

    /// Runs that were running in `before` and have finished now.
    pub fn newly_finished(&self, before: &HashSet<u64>) -> Vec<&Run> {
        self.finished
            .iter()
            .filter(|run| before.contains(&run.id))
            .collect()
    }

    /// PRs not in `before` (by URL).
    pub fn new_pull_requests(&self, before: &HashSet<String>) -> Vec<&PullRequest> {
        self.pull_requests
            .iter()
            .filter(|pr| !before.contains(&pr.url))
            .collect()
    }

    pub fn summary(&self) -> Summary {
        Summary {
            running: self.running.len(),
            failed: self.failed().count(),
            pull_requests: self.pull_requests.len(),
            errors: self.errors.len(),
        }
    }
}

fn fetch_prs(config: &AppConfig, now: DateTime<Utc>) -> anyhow::Result<Vec<PullRequest>> {
    let prs_config = &config.pull_requests;
    if prs_config.limit == 0 {
        return Ok(Vec::new());
    }
    let mut query = prs_config.query.clone();
    if prs_config.max_age_days > 0 {
        let since = now - Duration::days(i64::from(prs_config.max_age_days));
        query.push_str(&format!(" created:>={}", since.format("%Y-%m-%d")));
    }
    if !prs_config.drafts {
        query.push_str(" draft:false");
    }
    // Repo filters apply after the search, so fetch extra when they may drop results.
    let filtered = !config.repos.include.is_empty() || !config.repos.exclude.is_empty();
    let fetch = if filtered { 100 } else { prs_config.limit };
    let mut prs = github::search_prs(&query, fetch)?;
    prs.retain(|pr| config.repos.allows(&pr.repo));
    prs.truncate(prs_config.limit);
    Ok(prs)
}

/// Exact `include` entries, repos with open PRs, then discovered repos; filtered and deduped.
fn watched_repos(config: &AppConfig, prs: &[PullRequest], errors: &mut Vec<String>) -> Vec<String> {
    let repos = &config.repos;
    let mut candidates: Vec<String> = repos.exact_includes().map(str::to_string).collect();
    candidates.extend(prs.iter().map(|pr| pr.repo.clone()));
    if repos.discover > 0 {
        let mut owners: Vec<Option<&str>> = Vec::new();
        if repos.include.is_empty() {
            owners.push(None);
        }
        owners.extend(repos.included_owners().into_iter().map(Some));
        for owner in owners {
            match github::list_repos(owner, repos.discover) {
                Ok(found) => candidates.extend(found),
                Err(err) => errors.push(format!("{err:#}")),
            }
        }
    }
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|repo| repos.allows(repo) && seen.insert(repo.to_ascii_lowercase()))
        .collect()
}

fn classify(runs: Vec<Run>, now: DateTime<Utc>, recent_minutes: u32) -> (Vec<Run>, Vec<Run>) {
    let cutoff = now - Duration::minutes(i64::from(recent_minutes));
    let (mut running, mut finished): (Vec<Run>, Vec<Run>) =
        runs.into_iter().partition(Run::is_active);
    finished.retain(|run| recent_minutes > 0 && run.updated_at >= cutoff);
    running.sort_by_key(|run| std::cmp::Reverse(run.started_at.unwrap_or(run.updated_at)));
    finished.sort_by_key(|run| std::cmp::Reverse(run.updated_at));
    (running, finished)
}

/// Counts for `tyci status` and bar modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub running: usize,
    pub failed: usize,
    pub pull_requests: usize,
    pub errors: usize,
}

impl Report for Summary {
    fn format_plain(&self) -> String {
        let mut line = format!(
            "{} running, {} failed, {} open PRs",
            self.running, self.failed, self.pull_requests
        );
        if self.errors > 0 {
            line.push_str(&format!(" ({} gh errors, see tyci list)", self.errors));
        }
        line
    }

    fn format_polybar(&self) -> String {
        let mut parts = Vec::new();
        if self.failed > 0 {
            parts.push(format!("%{{F{}}}✗ {}%{{F-}}", ATTENTION_COLOR, self.failed));
        }
        if self.running > 0 {
            parts.push(format!("%{{F{}}}⟳ {}%{{F-}}", BUSY_COLOR, self.running));
        }
        parts.join(" ")
    }
}

/// "4m", "2h", "3d".
pub fn ago(from: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - from).num_seconds().max(0);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: u64, status: &str, conclusion: &str, minutes_ago: i64, now: DateTime<Utc>) -> Run {
        Run {
            id,
            repo: "me/x".into(),
            workflow: "CI".into(),
            title: "t".into(),
            branch: "main".into(),
            event: "push".into(),
            status: status.into(),
            conclusion: conclusion.into(),
            url: format!("u{id}"),
            started_at: None,
            updated_at: now - Duration::minutes(minutes_ago),
        }
    }

    #[test]
    fn splits_running_and_recent() {
        let now = Utc::now();
        let runs = vec![
            run(1, "in_progress", "", 1, now),
            run(2, "completed", "failure", 5, now),
            run(3, "completed", "success", 120, now),
            run(4, "queued", "", 0, now),
        ];
        let (running, finished) = classify(runs, now, 60);
        assert_eq!(running.iter().map(|r| r.id).collect::<Vec<_>>(), [4, 1]);
        assert_eq!(finished.iter().map(|r| r.id).collect::<Vec<_>>(), [2]);
        let (_, none) = classify(vec![run(5, "completed", "success", 0, now)], now, 0);
        assert!(none.is_empty());
    }

    #[test]
    fn detects_finished_runs() {
        let now = Utc::now();
        let snapshot = Snapshot {
            finished: vec![run(1, "completed", "success", 0, now)],
            ..Snapshot::default()
        };
        assert_eq!(snapshot.newly_finished(&HashSet::from([1])).len(), 1);
        assert!(snapshot.newly_finished(&HashSet::from([2])).is_empty());
    }

    #[test]
    fn polybar_is_empty_when_idle() {
        let idle = Summary {
            running: 0,
            failed: 0,
            pull_requests: 3,
            errors: 0,
        };
        assert_eq!(idle.format_polybar(), "");
        let busy = Summary { running: 2, ..idle };
        assert!(busy.format_polybar().contains("⟳ 2"));
    }

    #[test]
    fn formats_ages() {
        let now = Utc::now();
        assert_eq!(ago(now - Duration::seconds(5), now), "5s");
        assert_eq!(ago(now - Duration::minutes(4), now), "4m");
        assert_eq!(ago(now - Duration::hours(2), now), "2h");
        assert_eq!(ago(now - Duration::days(3), now), "3d");
    }
}
