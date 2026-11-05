//! Everything tyci knows about GitHub comes from the `gh` CLI, so it uses your existing
//! `gh auth login` (or `GH_TOKEN`) and needs no token of its own.

use std::process::Command;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PullRequest {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub author: String,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Run {
    pub id: u64,
    pub repo: String,
    pub workflow: String,
    pub title: String,
    pub branch: String,
    pub event: String,
    /// `queued`, `in_progress`, `waiting`, `requested`, `pending` or `completed`.
    pub status: String,
    /// Set once completed: `success`, `failure`, `cancelled`, `skipped`, ...
    pub conclusion: String,
    pub url: String,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl Run {
    pub fn is_active(&self) -> bool {
        self.status != "completed"
    }

    pub fn failed(&self) -> bool {
        matches!(
            self.conclusion.as_str(),
            "failure" | "timed_out" | "startup_failure" | "action_required"
        )
    }

    /// One-character status mark for menus and notifications.
    pub fn mark(&self) -> &'static str {
        match (self.status.as_str(), self.conclusion.as_str()) {
            ("in_progress", _) => "⟳",
            ("completed", "success") => "✓",
            ("completed", "cancelled" | "skipped" | "neutral" | "stale") => "–",
            ("completed", _) => "✗",
            _ => "…",
        }
    }
}

fn gh(args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("gh")
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .output()
        .context("failed to run gh (is the GitHub CLI installed?)")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        bail!("gh {} {}: {}", args[0], args[1], line.trim());
    }
    Ok(output.stdout)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPr {
    number: u64,
    title: String,
    url: String,
    repository: RawRepoRef,
    author: RawAuthor,
    created_at: DateTime<Utc>,
    #[serde(default)]
    is_draft: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepoRef {
    name_with_owner: String,
}

#[derive(Deserialize)]
struct RawAuthor {
    login: String,
}

/// Open PRs matching `query` (GitHub search qualifiers), newest first.
pub fn search_prs(query: &str, limit: usize) -> Result<Vec<PullRequest>> {
    let limit = limit.clamp(1, 100).to_string();
    let mut args = vec![
        "search",
        "prs",
        "--state",
        "open",
        "--sort",
        "created",
        "--order",
        "desc",
        "--limit",
        &limit,
        "--json",
        "number,title,url,repository,author,createdAt,isDraft",
        "--",
    ];
    args.extend(query.split_whitespace());
    parse_prs(&gh(&args)?)
}

fn parse_prs(json: &[u8]) -> Result<Vec<PullRequest>> {
    let raw: Vec<RawPr> = serde_json::from_slice(json).context("unexpected gh search output")?;
    Ok(raw
        .into_iter()
        .map(|pr| PullRequest {
            repo: pr.repository.name_with_owner,
            number: pr.number,
            title: pr.title,
            author: pr.author.login,
            url: pr.url,
            created_at: pr.created_at,
            draft: pr.is_draft,
        })
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepo {
    name_with_owner: String,
    #[serde(default)]
    is_archived: bool,
}

/// `owner`'s (or your own) most recently pushed, unarchived repos.
pub fn list_repos(owner: Option<&str>, limit: usize) -> Result<Vec<String>> {
    let limit = limit.to_string();
    let mut args = vec!["repo", "list"];
    args.extend(owner);
    args.extend([
        "--no-archived",
        "--limit",
        &limit,
        "--json",
        "nameWithOwner,isArchived",
    ]);
    let raw: Vec<RawRepo> =
        serde_json::from_slice(&gh(&args)?).context("unexpected gh repo list output")?;
    Ok(raw
        .into_iter()
        .filter(|r| !r.is_archived)
        .map(|r| r.name_with_owner)
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRun {
    database_id: u64,
    #[serde(default)]
    workflow_name: String,
    #[serde(default)]
    display_title: String,
    #[serde(default)]
    head_branch: String,
    #[serde(default)]
    event: String,
    status: String,
    #[serde(default)]
    conclusion: String,
    url: String,
    started_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

/// The latest `limit` workflow runs in `repo`.
pub fn list_runs(repo: &str, limit: usize) -> Result<Vec<Run>> {
    let limit = limit.max(1).to_string();
    let json = gh(&[
        "run",
        "list",
        "--repo",
        repo,
        "--limit",
        &limit,
        "--json",
        "databaseId,workflowName,displayTitle,headBranch,event,status,conclusion,url,startedAt,updatedAt",
    ])?;
    parse_runs(repo, &json)
}

fn parse_runs(repo: &str, json: &[u8]) -> Result<Vec<Run>> {
    let raw: Vec<RawRun> = serde_json::from_slice(json).context("unexpected gh run output")?;
    Ok(raw
        .into_iter()
        .map(|run| Run {
            id: run.database_id,
            repo: repo.to_string(),
            workflow: run.workflow_name,
            title: run.display_title,
            branch: run.head_branch,
            event: run.event,
            status: run.status,
            conclusion: run.conclusion,
            url: run.url,
            // Queued runs report the zero time.
            started_at: run.started_at.filter(|t| t.year() > 2000),
            updated_at: run.updated_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_output() {
        let json = br#"[{"author":{"login":"me","is_bot":false},"createdAt":"2026-11-04T23:20:34Z",
            "isDraft":true,"number":483,"repository":{"name":"x","nameWithOwner":"me/x"},
            "title":"Add a note","url":"https://github.com/me/x/pull/483"}]"#;
        let prs = parse_prs(json).unwrap();
        assert_eq!(prs[0].repo, "me/x");
        assert_eq!(prs[0].author, "me");
        assert!(prs[0].draft);
    }

    #[test]
    fn parses_runs_and_drops_zero_start_times() {
        let json = br#"[{"conclusion":"","databaseId":1,"displayTitle":"t","event":"push",
            "headBranch":"main","status":"queued","url":"u","workflowName":"CI",
            "startedAt":"0001-01-01T00:00:00Z","updatedAt":"2026-11-05T01:01:37Z"},
            {"conclusion":"failure","databaseId":2,"displayTitle":"t","event":"push",
            "headBranch":"main","status":"completed","url":"u","workflowName":"CI",
            "startedAt":"2026-11-05T01:00:00Z","updatedAt":"2026-11-05T01:01:37Z"}]"#;
        let runs = parse_runs("me/x", json).unwrap();
        assert!(runs[0].is_active());
        assert_eq!(runs[0].started_at, None);
        assert!(runs[1].failed());
        assert_eq!(runs[1].mark(), "✗");
    }
}
