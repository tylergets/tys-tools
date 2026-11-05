//! Sending screenshots to opencode sessions through the local opencode service.
//!
//! opencode (v2) runs a background service that its terminal UIs connect to. The service
//! records its URL and password in `$XDG_STATE_HOME/opencode/service.json`; its HTTP API lists
//! sessions and accepts prompts with image attachments. A screenshot is sent as an empty
//! prompt with the image attached. By default it doesn't start the agent: it waits in the
//! session's inbox and goes to the model together with the next message typed in that
//! session, so the screenshot can be followed by a question about it.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::Deserialize;
use serde_json::json;

/// HTTP Basic auth user name; the password is per service.
const USER: &str = "opencode";
const TIMEOUT: Duration = Duration::from_secs(3);
/// Sessions fetched, before sorting by last activity.
const FETCH: usize = 30;

/// A connection to the running opencode service.
pub struct Service {
    url: String,
    authorization: String,
    agent: ureq::Agent,
}

/// An opencode session (top-level, not archived).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub directory: Option<PathBuf>,
    /// Last activity, milliseconds since the epoch.
    pub updated: i64,
    /// The agent is working in it right now.
    pub running: bool,
}

impl Session {
    /// `~/src/app · 5m ago`, plus ` · running` while the agent is working.
    pub fn detail(&self, now_ms: i64) -> String {
        let mut detail = match &self.directory {
            Some(dir) => format!("{} · {}", tilde(dir), ago(now_ms - self.updated)),
            None => ago(now_ms - self.updated),
        };
        if self.running {
            detail.push_str(" · running");
        }
        detail
    }
}

/// Where to send a screenshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Session(Session),
    /// A new session working in this directory, opened in a terminal.
    NewSession(PathBuf),
}

impl Target {
    /// The picker's row: a title and a detail line.
    pub fn describe(&self, now_ms: i64) -> (String, String) {
        match self {
            Self::Session(session) => (session.title.clone(), session.detail(now_ms)),
            Self::NewSession(dir) => ("+ New session".to_string(), tilde(dir)),
        }
    }
}

/// The picker's choices: `sessions` (most recently active first), then a new session in each
/// of their most recent distinct directories, at most `new_max` (the home directory if there
/// are no sessions at all).
pub fn targets(sessions: Vec<Session>, new_max: usize) -> Vec<Target> {
    let mut directories: Vec<PathBuf> = Vec::new();
    for dir in sessions
        .iter()
        .filter_map(|session| session.directory.clone())
    {
        if directories.len() < new_max && !directories.contains(&dir) {
            directories.push(dir);
        }
    }
    if directories.is_empty() {
        directories.extend(dirs::home_dir().filter(|_| new_max > 0));
    }
    sessions
        .into_iter()
        .map(Target::Session)
        .chain(directories.into_iter().map(Target::NewSession))
        .collect()
}

/// Open `opencode --session <id>` in a new `terminal` window (kitty command-line syntax)
/// working in `directory`. Returns once the terminal is started; it is not waited for.
pub fn open_in_terminal(terminal: &str, session_id: &str, directory: &Path) -> Result<()> {
    Command::new(terminal)
        .arg("--directory")
        .arg(directory)
        .args(["opencode", "--session", session_id])
        .arg(directory)
        .current_dir(directory)
        // tyshot's wrapper points this at its own GTK libraries; the terminal brings its own.
        .env_remove("LD_LIBRARY_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own process group, so it outlives tyshot and Ctrl+C in a parent terminal.
        .process_group(0)
        .spawn()
        .with_context(|| format!("failed to start the terminal `{terminal}`"))?;
    Ok(())
}

#[derive(Deserialize)]
struct ServiceState {
    url: String,
    password: String,
}

#[derive(Deserialize)]
struct Page<T> {
    data: T,
}

#[derive(Deserialize)]
struct SessionInfo {
    id: String,
    title: Option<String>,
    time: SessionTime,
    location: Option<Location>,
}

#[derive(Deserialize)]
struct SessionTime {
    updated: f64,
}

#[derive(Deserialize)]
struct Location {
    directory: PathBuf,
}

impl Service {
    /// Connect to the service recorded in `service.json`. Fails with an explanation if
    /// opencode's service has never run.
    pub fn discover() -> Result<Self> {
        let path = dirs::state_dir()
            .context("unable to resolve the XDG state directory")?
            .join("opencode/service.json");
        let raw = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "opencode service not found ({} is missing); start opencode or run \
                 `opencode service start`",
                path.display()
            )
        })?;
        let state: ServiceState = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let credentials =
            base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{}", state.password));
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self {
            url: state.url.trim_end_matches('/').to_string(),
            authorization: format!("Basic {credentials}"),
            agent,
        })
    }

    /// Top-level, unarchived sessions, most recently active first, at most `limit`.
    pub fn sessions(&self, limit: usize) -> Result<Vec<Session>> {
        let mut response = self
            .agent
            .get(format!("{}/api/session", self.url))
            .header("Authorization", &self.authorization)
            .query("limit", FETCH.to_string())
            .query("parentID", "null")
            .query("archived", "false")
            .call()
            .map_err(|err| self.unreachable(err))?;
        check(&mut response)?;
        let page: Page<Vec<SessionInfo>> = response
            .body_mut()
            .read_json()
            .context("unexpected session list from opencode")?;

        // Which sessions the agent is working in; only decoration, so failures are ignored.
        let running: Vec<String> = self
            .agent
            .get(format!("{}/api/session/active", self.url))
            .header("Authorization", &self.authorization)
            .call()
            .ok()
            .filter(|response| response.status().is_success())
            .and_then(|mut response| {
                response
                    .body_mut()
                    .read_json::<Page<serde_json::Map<String, serde_json::Value>>>()
                    .ok()
            })
            .map(|page| page.data.into_iter().map(|(id, _)| id).collect())
            .unwrap_or_default();

        let mut sessions: Vec<Session> = page
            .data
            .into_iter()
            .map(|info| session_from(info, &running))
            .collect();
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated));
        sessions.truncate(limit);
        Ok(sessions)
    }

    /// Create a top-level session working in `directory`.
    pub fn create_session(&self, directory: &Path) -> Result<Session> {
        let mut response = self
            .agent
            .post(format!("{}/api/session", self.url))
            .header("Authorization", &self.authorization)
            .send_json(json!({ "location": { "directory": directory } }))
            .map_err(|err| self.unreachable(err))?;
        check(&mut response)?;
        let created: Page<SessionInfo> = response
            .body_mut()
            .read_json()
            .context("unexpected reply from opencode when creating a session")?;
        Ok(session_from(created.data, &[]))
    }

    /// Attach `png` to the session as `name`. With `submit`, the agent starts on it right
    /// away; otherwise it waits and is sent with the next message typed in the session.
    pub fn send_image(&self, session_id: &str, png: &[u8], name: &str, submit: bool) -> Result<()> {
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        let body = json!({
            "text": "",
            "files": [{ "uri": format!("data:image/png;base64,{data}"), "name": name }],
            "resume": submit,
        });
        let mut response = self
            .agent
            .post(format!("{}/api/session/{session_id}/prompt", self.url))
            .header("Authorization", &self.authorization)
            .send_json(body)
            .map_err(|err| self.unreachable(err))?;
        // The reply echoes the whole image back; only the status matters.
        check(&mut response)
    }

    fn unreachable(&self, err: ureq::Error) -> anyhow::Error {
        anyhow::Error::new(err).context(format!(
            "opencode service is not running at {}; start opencode or run \
             `opencode service start`",
            self.url
        ))
    }
}

fn session_from(info: SessionInfo, running: &[String]) -> Session {
    Session {
        running: running.contains(&info.id),
        title: info
            .title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "Untitled session".to_string()),
        directory: info.location.map(|location| location.directory),
        updated: info.time.updated as i64,
        id: info.id,
    }
}

/// Turn an error status into an error with the server's message.
fn check(response: &mut ureq::http::Response<ureq::Body>) -> Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let message = response
        .body_mut()
        .with_config()
        .limit(16 * 1024)
        .read_to_string()
        .unwrap_or_default();
    if status == 401 {
        bail!("opencode rejected the service password (HTTP 401)");
    }
    bail!("opencode returned HTTP {status}: {}", message.trim());
}

fn ago(ms: i64) -> String {
    let seconds = ms.max(0) / 1000;
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}

fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(PathBuf::from)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_list() {
        let raw = r#"{"data":[
            {"id":"ses_a","projectID":"p","title":"Fix the build","cost":0,"tokens":{},
             "time":{"created":1,"updated":1793818875925.0},
             "location":{"directory":"/src/app"}},
            {"id":"ses_b","projectID":"p","cost":0,"tokens":{},"time":{"created":1,"updated":2},
             "location":{"directory":"/src/other"}}
        ],"cursor":{"previous":null,"next":null}}"#;
        let page: Page<Vec<SessionInfo>> = serde_json::from_str(raw).unwrap();
        assert_eq!(page.data.len(), 2);
        assert_eq!(page.data[0].title.as_deref(), Some("Fix the build"));
        assert_eq!(page.data[0].time.updated as i64, 1793818875925);
        assert!(page.data[1].title.is_none());
    }

    fn session(id: &str, dir: &str) -> Session {
        Session {
            id: id.into(),
            title: id.into(),
            directory: Some(PathBuf::from(dir)),
            updated: 0,
            running: false,
        }
    }

    #[test]
    fn offers_new_sessions_in_recent_directories() {
        let targets = targets(
            vec![
                session("a", "/src/app"),
                session("b", "/src/app"),
                session("c", "/src/lib"),
                session("d", "/src/web"),
            ],
            2,
        );
        assert_eq!(targets.len(), 6);
        assert_eq!(targets[4], Target::NewSession(PathBuf::from("/src/app")));
        assert_eq!(targets[5], Target::NewSession(PathBuf::from("/src/lib")));
        assert_eq!(
            targets[5].describe(0),
            ("+ New session".to_string(), "/src/lib".to_string())
        );
    }

    #[test]
    fn describes_sessions() {
        let session = Session {
            id: "ses_a".into(),
            title: "Fix the build".into(),
            directory: Some(PathBuf::from("/src/app")),
            updated: 1_000_000,
            running: false,
        };
        assert_eq!(session.detail(1_000_000 + 5_000), "/src/app · just now");
        assert_eq!(session.detail(1_000_000 + 300_000), "/src/app · 5m ago");
        let running = Session {
            running: true,
            directory: None,
            ..session
        };
        assert_eq!(running.detail(1_000_000 + 7_200_000), "2h ago · running");
    }
}
