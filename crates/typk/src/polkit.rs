//! Talking to polkitd over the system bus: registering as the session's authentication
//! agent, the requests it sends us, and asking for an authorization ourselves (`typk test`).
//!
//! polkitd picks the agent by login session, so we register for the session polkitd will
//! resolve us to: the one our process is in, or (from a systemd user service, which is in no
//! session) the user's display session, as logind reports it.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
use dbus::blocking::LocalConnection;
use dbus::Message;

pub type Bus = LocalConnection;

const AUTHORITY: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY_IFACE: &str = "org.freedesktop.PolicyKit1.Authority";
/// Where polkitd calls us.
pub const AGENT_PATH: &str = "/org/freedesktop/PolicyKit1/AuthenticationAgent";
/// The error to return from `BeginAuthentication` when the prompt was dismissed.
pub const ERROR_CANCELLED: &str = "org.freedesktop.PolicyKit1.Error.Cancelled";
const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const TIMEOUT: Duration = Duration::from_secs(25);
/// `POLKIT_CHECK_AUTHORIZATION_FLAGS_ALLOW_USER_INTERACTION`.
const ALLOW_USER_INTERACTION: u32 = 1;
/// typk's own action (auth_self), from `typk.policy`.
pub const VERIFY_ACTION: &str = "tools.tys.typk.verify";
/// Asked for instead when polkit doesn't know [`VERIFY_ACTION`]; needs an admin.
pub const FALLBACK_ACTION: &str = "org.freedesktop.policykit.exec";
/// polkitd adds the asking process's pid to the details it gives the agent; `typk verify`
/// tells the agent its reason by pid (polkitd lets only root pass details through).
pub const SUBJECT_PID_DETAIL: &str = "polkit.subject-pid";
/// How long `typk verify` waits for the prompt to be answered.
const TEST_TIMEOUT: Duration = Duration::from_secs(600);

/// Who an action lets authenticate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    User(u32),
    Group(u32),
    /// A kind we don't handle (netgroups).
    Other(String),
}

/// One `BeginAuthentication` call from polkitd.
#[derive(Debug, Clone)]
pub struct Request {
    pub action_id: String,
    /// Why authentication is needed, already localized by polkitd.
    pub message: String,
    pub icon_name: String,
    pub details: BTreeMap<String, String>,
    pub cookie: String,
    pub identities: Vec<Identity>,
}

impl Request {
    pub fn parse(msg: &Message) -> Result<Self> {
        let mut args = msg.iter_init();
        let action_id: String = args.read()?;
        let message: String = args.read()?;
        let icon_name: String = args.read()?;
        let details: HashMap<String, String> = args.read()?;
        let cookie: String = args.read()?;
        let identities: Vec<(String, PropMap)> = args.read()?;
        let identities = identities
            .into_iter()
            .map(|(kind, props)| {
                let id = |key: &str| {
                    props
                        .get(key)
                        .and_then(|v| v.0.as_u64().or_else(|| v.0.as_i64().map(|n| n as u64)))
                        .map(|n| n as u32)
                };
                match (kind.as_str(), id("uid"), id("gid")) {
                    ("unix-user", Some(uid), _) => Identity::User(uid),
                    ("unix-group", _, Some(gid)) => Identity::Group(gid),
                    _ => Identity::Other(kind),
                }
            })
            .collect();
        Ok(Self {
            action_id,
            message,
            icon_name,
            details: details.into_iter().collect(),
            cookie,
            identities,
        })
    }
}

/// A polkit subject: who an agent serves, or who asks for an authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Session(String),
    Process { pid: u32, start_time: u64 },
}

impl Subject {
    /// The login session polkitd will attribute this process to.
    pub fn for_session(conn: &Bus) -> Result<Self> {
        Ok(Subject::Session(session_id(conn)?))
    }

    pub fn this_process() -> Result<Self> {
        Ok(Subject::Process {
            pid: std::process::id(),
            start_time: process_start_time()?,
        })
    }

    /// The D-Bus `(sa{sv})` form.
    fn to_arg(&self) -> (String, PropMap) {
        let mut props = PropMap::new();
        let kind = match self {
            Subject::Session(id) => {
                props.insert("session-id".into(), Variant(Box::new(id.clone())));
                "unix-session"
            }
            Subject::Process { pid, start_time } => {
                props.insert("pid".into(), Variant(Box::new(*pid)));
                props.insert("start-time".into(), Variant(Box::new(*start_time)));
                "unix-process"
            }
        };
        (kind.to_string(), props)
    }
}

impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Subject::Session(id) => write!(f, "session {id}"),
            Subject::Process { pid, .. } => write!(f, "process {pid}"),
        }
    }
}

/// Our session id as polkitd computes it: the session this process is in, else the user's
/// display session (systemd user services are in no session).
fn session_id(conn: &Bus) -> Result<String> {
    let manager = conn.with_proxy(LOGIND, LOGIND_PATH, TIMEOUT);
    let by_pid: Result<(dbus::Path<'static>,), _> = manager.method_call(
        "org.freedesktop.login1.Manager",
        "GetSessionByPID",
        (std::process::id(),),
    );
    if let Ok((path,)) = by_pid {
        let session = conn.with_proxy(LOGIND, path, TIMEOUT);
        return session
            .get("org.freedesktop.login1.Session", "Id")
            .context("failed to read the session id from logind");
    }
    let uid = crate::users::current_uid();
    let user: Result<(dbus::Path<'static>,), _> =
        manager.method_call("org.freedesktop.login1.Manager", "GetUser", (uid,));
    match user {
        Ok((path,)) => {
            let user = conn.with_proxy(LOGIND, path, TIMEOUT);
            let (id, _path): (String, dbus::Path<'static>) = user
                .get("org.freedesktop.login1.User", "Display")
                .context("failed to read the display session from logind")?;
            if !id.is_empty() {
                return Ok(id);
            }
            bail!("logind reports no graphical session for this user; is a session logged in?");
        }
        Err(err) => {
            if let Ok(id) = std::env::var("XDG_SESSION_ID") {
                return Ok(id);
            }
            bail!("cannot find the login session (logind: {err}) and XDG_SESSION_ID is unset");
        }
    }
}

/// Field 22 of `/proc/self/stat`, in clock ticks since boot; polkit uses it to tell a pid
/// from a recycled one.
fn process_start_time() -> Result<u64> {
    let stat =
        std::fs::read_to_string("/proc/self/stat").context("failed to read /proc/self/stat")?;
    let after_comm = stat
        .rsplit_once(')')
        .map(|(_, rest)| rest)
        .context("malformed /proc/self/stat")?;
    // `after_comm` starts at field 3 (state), so start time (field 22) is the 20th.
    after_comm
        .split_whitespace()
        .nth(19)
        .and_then(|s| s.parse().ok())
        .context("no start time in /proc/self/stat")
}

/// The locale polkitd should translate messages to.
fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| "C".to_string())
}

/// Register as the authentication agent for `subject`. Fails if one exists already.
pub fn register(conn: &Bus, subject: &Subject) -> Result<()> {
    let authority = conn.with_proxy(AUTHORITY, AUTHORITY_PATH, TIMEOUT);
    authority
        .method_call::<(), _, _, _>(
            AUTHORITY_IFACE,
            "RegisterAuthenticationAgent",
            (subject.to_arg(), locale(), AGENT_PATH),
        )
        .map_err(|err| {
            let message = err.message().unwrap_or("unknown error").to_string();
            if message.contains("already exists") {
                anyhow::anyhow!(
                    "polkit already has an authentication agent for this {subject} \
                     (polkit-gnome, lxpolkit, hyprpolkitagent, ...); stop it to use typk"
                )
            } else {
                anyhow::anyhow!("failed to register with polkit for {subject}: {message}")
            }
        })
}

pub fn unregister(conn: &Bus, subject: &Subject) {
    let authority = conn.with_proxy(AUTHORITY, AUTHORITY_PATH, TIMEOUT);
    if let Err(err) = authority.method_call::<(), _, _, _>(
        AUTHORITY_IFACE,
        "UnregisterAuthenticationAgent",
        (subject.to_arg(), AGENT_PATH),
    ) {
        eprintln!("warning: failed to unregister from polkit: {err}");
    }
}

/// The result of `typk verify`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Authorized,
    NotAuthorized,
    /// The action needs authentication but no agent prompted for it.
    NoAgent,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Outcome::Authorized => "Authorized",
            Outcome::NotAuthorized => "Not authorized",
            Outcome::NoAgent => {
                "Not authorized: authentication is needed but no agent is registered for this \
                 session (is `typk run` running?)"
            }
        })
    }
}

/// Ask polkitd whether this process may do `action_id`, letting it prompt through the
/// session's agent. Blocks until the prompt is answered.
pub fn check_authorization(action_id: &str) -> Result<Outcome> {
    let conn = Bus::new_system().context("failed to connect to the system bus")?;
    let authority = conn.with_proxy(AUTHORITY, AUTHORITY_PATH, TEST_TIMEOUT);
    let ((authorized, challenge, _details),): ((bool, bool, HashMap<String, String>),) = authority
        .method_call(
            AUTHORITY_IFACE,
            "CheckAuthorization",
            (
                Subject::this_process()?.to_arg(),
                action_id,
                HashMap::<&str, &str>::new(),
                ALLOW_USER_INTERACTION,
                "",
            ),
        )
        .context("CheckAuthorization failed")?;
    Ok(if authorized {
        Outcome::Authorized
    } else if challenge {
        Outcome::NoAgent
    } else {
        Outcome::NotAuthorized
    })
}

/// `typk verify`: tell the agent why (shown in the prompt), then ask for `action`. When
/// polkit doesn't know typk's own action (it isn't installed system-wide), ask for
/// [`FALLBACK_ACTION`] instead.
pub fn verify(action: &str, reason: Option<&str>, app: Option<&str>) -> Result<Outcome> {
    crate::reasons::announce(reason, app);
    match check_authorization(action) {
        Err(err) if action == VERIFY_ACTION && format!("{err:#}").contains("not registered") => {
            eprintln!(
                "warning: polkit doesn't know {VERIFY_ACTION} (typk isn't installed system-wide; \
                 see the README); asking for {FALLBACK_ACTION} instead, which needs an admin"
            );
            check_authorization(FALLBACK_ACTION)
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_own_start_time() {
        assert!(process_start_time().unwrap() > 0);
    }

    #[test]
    fn subject_args() {
        let (kind, props) = Subject::Session("3".into()).to_arg();
        assert_eq!(kind, "unix-session");
        assert_eq!(props["session-id"].0.as_str(), Some("3"));
        let (kind, props) = Subject::Process {
            pid: 7,
            start_time: 9,
        }
        .to_arg();
        assert_eq!(kind, "unix-process");
        assert_eq!(props["pid"].0.as_u64(), Some(7));
        assert_eq!(props["start-time"].0.as_u64(), Some(9));
    }
}
