//! Tray applet: running and recently finished CI runs, open PRs, each opening in the browser.
//! Refreshes run `gh` on a background thread; the menu is rebuilt from the latest snapshot.

use std::cell::RefCell;
use std::collections::HashSet;
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use gtk::glib::{self, ControlFlow};
use ty_common::icons::Tool;
use ty_common::tray::{self, Applet, IconState, MenuEntry, TraySpec};

use crate::config::{AppConfig, APP_NAME};
use crate::github::{PullRequest, Run};
use crate::snapshot::{ago, Snapshot};

const ACTION_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Keeps the "4m" ages in the menu current between refreshes.
const RELABEL_INTERVAL: Duration = Duration::from_secs(30);
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(8);
const TITLE_CHARS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayAction {
    Open(String),
    Refresh,
    Quit,
}

#[derive(Default)]
struct State {
    snapshot: Snapshot,
    fetching: bool,
    /// Runs seen running, to notify when they finish.
    running: HashSet<u64>,
    /// PR URLs already seen; `None` until the first refresh, so startup doesn't notify.
    seen_prs: Option<HashSet<String>>,
}

struct App {
    config: AppConfig,
    state: RefCell<State>,
    applet: RefCell<Applet<TrayAction>>,
}

pub fn run(config: AppConfig) -> Result<()> {
    gtk::init().context("failed to initialize GTK")?;
    let (tx, rx) = mpsc::channel();
    let applet = Applet::new(
        APP_NAME,
        &Tool::Ci.tray_icons(),
        spec(&Snapshot::default(), true, Utc::now()),
        tx,
    )?;
    let app = Rc::new(App {
        config,
        state: RefCell::new(State::default()),
        applet: RefCell::new(applet),
    });

    refresh(&app);
    let poller = app.clone();
    glib::timeout_add_local(Duration::from_secs(app.config.poll_seconds), move || {
        refresh(&poller);
        ControlFlow::Continue
    });
    let relabel = app.clone();
    glib::timeout_add_local(RELABEL_INTERVAL, move || {
        relabel.redraw();
        ControlFlow::Continue
    });

    let handler = app.clone();
    tray::drain_actions(rx, ACTION_POLL_INTERVAL, move |action| {
        match action {
            TrayAction::Open(url) => open_url(&url),
            TrayAction::Refresh => refresh(&handler),
            TrayAction::Quit => {
                gtk::main_quit();
                return ControlFlow::Break;
            }
        }
        ControlFlow::Continue
    });

    gtk::main();
    Ok(())
}

impl App {
    fn redraw(&self) {
        let state = self.state.borrow();
        self.applet
            .borrow_mut()
            .update(spec(&state.snapshot, state.fetching, Utc::now()));
    }
}

fn refresh(app: &Rc<App>) {
    {
        let mut state = app.state.borrow_mut();
        if state.fetching {
            return;
        }
        state.fetching = true;
    }
    app.redraw();

    let config = app.config.clone();
    let app = app.clone();
    tray::spawn_blocking(
        move || Snapshot::fetch(&config),
        move |snapshot| {
            for err in &snapshot.errors {
                eprintln!("tyci: {err}");
            }
            notify_changes(&app.config, &app.state.borrow(), &snapshot);
            {
                let mut state = app.state.borrow_mut();
                state.running = snapshot.running_ids();
                state.seen_prs = Some(
                    snapshot
                        .pull_requests
                        .iter()
                        .map(|pr| pr.url.clone())
                        .chain(state.seen_prs.take().unwrap_or_default())
                        .collect(),
                );
                state.snapshot = snapshot;
                state.fetching = false;
            }
            app.redraw();
        },
    );
}

fn notify_changes(config: &AppConfig, state: &State, snapshot: &Snapshot) {
    let notifications = &config.notifications;
    if notifications.on_finish {
        for run in snapshot.newly_finished(&state.running) {
            if notifications.only_failures && !run.failed() {
                continue;
            }
            let outcome = match run.conclusion.as_str() {
                "success" => "passed".to_string(),
                "failure" => "failed".to_string(),
                other => other.replace('_', " "),
            };
            notify(
                &format!("{} {} {outcome}", run.mark(), run.workflow),
                &format!("{} · {}\n{}", run.repo, run.branch, run.title),
                &run.url,
            );
        }
    }
    if let (true, Some(seen)) = (notifications.on_new_pr, &state.seen_prs) {
        for pr in snapshot.new_pull_requests(seen) {
            notify(
                &format!("New PR in {}", pr.repo),
                &format!("#{} {}\nby {}", pr.number, pr.title, pr.author),
                &pr.url,
            );
        }
    }
}

fn notify(summary: &str, body: &str, url: &str) {
    let url = url.to_string();
    tray::notify_with_actions(
        APP_NAME,
        summary,
        body,
        NOTIFY_TIMEOUT,
        &[("default", "Open")],
        move |_| open_url(&url),
    );
}

fn open_url(url: &str) {
    match Command::new("xdg-open").arg(url).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => eprintln!("error: failed to launch xdg-open: {err}"),
    }
}

fn spec(snapshot: &Snapshot, fetching: bool, now: DateTime<Utc>) -> TraySpec<TrayAction> {
    let summary = snapshot.summary();
    let header = if snapshot.fetched_at.is_none() {
        "Loading…".to_string()
    } else {
        let mut parts = vec![format!("{} running", summary.running)];
        if summary.failed > 0 {
            parts.push(format!("{} failed", summary.failed));
        }
        parts.push(format!("{} PRs", summary.pull_requests));
        parts.join(" · ")
    };

    let mut menu = vec![MenuEntry::Label(header.clone()), MenuEntry::Separator];

    if !snapshot.running.is_empty() {
        menu.push(MenuEntry::Label("Running".into()));
        menu.extend(snapshot.running.iter().map(|run| run_entry(run, now)));
    }
    if !snapshot.finished.is_empty() {
        menu.push(MenuEntry::Label("Finished".into()));
        menu.extend(snapshot.finished.iter().map(|run| run_entry(run, now)));
    }
    if snapshot.running.is_empty() && snapshot.finished.is_empty() && snapshot.fetched_at.is_some()
    {
        menu.push(MenuEntry::Label("No recent CI runs".into()));
    }

    menu.push(MenuEntry::Separator);
    menu.push(MenuEntry::Label("Pull requests".into()));
    if snapshot.pull_requests.is_empty() && snapshot.fetched_at.is_some() {
        menu.push(MenuEntry::Label("No open PRs".into()));
    }
    menu.extend(snapshot.pull_requests.iter().map(|pr| pr_entry(pr, now)));

    if !snapshot.errors.is_empty() {
        menu.push(MenuEntry::Separator);
        menu.push(MenuEntry::Submenu {
            label: format!("⚠ {} gh errors", snapshot.errors.len()),
            entries: snapshot
                .errors
                .iter()
                .map(|err| MenuEntry::Label(truncate(err, 100)))
                .collect(),
        });
    }

    menu.push(MenuEntry::Separator);
    menu.push(MenuEntry::Action {
        label: if fetching { "Refreshing…" } else { "Refresh" }.into(),
        enabled: !fetching,
        action: TrayAction::Refresh,
    });
    menu.push(MenuEntry::Action {
        label: "Quit".into(),
        enabled: true,
        action: TrayAction::Quit,
    });

    let icon = if summary.failed > 0 {
        IconState::Attention
    } else if summary.running > 0 {
        IconState::Busy
    } else {
        IconState::Normal
    };
    TraySpec {
        icon,
        title: format!("{APP_NAME}: {header}"),
        menu,
        click_action: Some(TrayAction::Refresh),
    }
}

fn run_entry(run: &Run, now: DateTime<Utc>) -> MenuEntry<TrayAction> {
    let time = match (run.is_active(), run.started_at) {
        (true, Some(started)) => ago(started, now),
        (true, None) => format!(
            "{} {}",
            run.status.replace('_', " "),
            ago(run.updated_at, now)
        ),
        (false, _) => format!("{} ago", ago(run.updated_at, now)),
    };
    MenuEntry::Action {
        label: format!(
            "{} {} · {} · {} · {time}",
            run.mark(),
            short_repo(&run.repo),
            truncate(&run.workflow, 30),
            truncate(&run.branch, 30),
        ),
        enabled: true,
        action: TrayAction::Open(run.url.clone()),
    }
}

fn pr_entry(pr: &PullRequest, now: DateTime<Utc>) -> MenuEntry<TrayAction> {
    let draft = if pr.draft { " (draft)" } else { "" };
    MenuEntry::Action {
        label: format!(
            "{}#{} {}{draft} · {} · {}",
            short_repo(&pr.repo),
            pr.number,
            truncate(&pr.title, TITLE_CHARS),
            pr.author,
            ago(pr.created_at, now),
        ),
        enabled: true,
        action: TrayAction::Open(pr.url.clone()),
    }
}

fn short_repo(repo: &str) -> &str {
    repo.split_once('/').map_or(repo, |(_, name)| name)
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_on_char_boundaries() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("ééééé", 3), "éé…");
    }

    #[test]
    fn refresh_is_a_top_level_action() {
        let spec = spec(&Snapshot::default(), false, Utc::now());
        let click = spec.click_action.clone().unwrap();
        assert!(spec
            .menu
            .iter()
            .any(|entry| matches!(entry, MenuEntry::Action { action, .. } if *action == click)));
        assert_eq!(spec.icon, IconState::Normal);
    }
}
