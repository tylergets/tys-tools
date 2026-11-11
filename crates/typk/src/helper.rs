//! One authentication attempt through polkit's `polkit-agent-helper-1`. The helper runs
//! the `polkit-1` PAM stack as root, so whatever it is set up for (a fingerprint reader via
//! pam_fprintd, a password via pam_unix, ...) works, and on success it tells polkitd itself.
//! We only relay the conversation.
//!
//! polkit 125 and later run the helper as a socket-activated systemd service; older setups
//! have a setuid binary, which we spawn. The protocol is the same: we send the user name
//! (socket only) and the cookie, one per line; the helper sends `PAM_PROMPT_ECHO_OFF <text>`,
//! `PAM_PROMPT_ECHO_ON <text>`, `PAM_ERROR_MSG <text>` and `PAM_TEXT_INFO <text>` (the text
//! C-escaped, as by `g_strescape`), then `SUCCESS` or `FAILURE`. Answers are written as-is,
//! one per line.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;

use anyhow::{Context, Result};
use ty_common::tray::Sender;

/// polkit's helper socket (polkit >= 125 with systemd).
pub const SOCKET: &str = "/run/polkit/agent-helper.socket";

/// Where the setuid helper lives on systems without the socket.
const HELPER_PATHS: &[&str] = &[
    // NixOS before polkit 125.
    "/run/wrappers/bin/polkit-agent-helper-1",
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-agent-helper-1",
    // Debian and Ubuntu.
    "/usr/lib/policykit-1/polkit-agent-helper-1",
];

/// What the helper (PAM) says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// PAM wants an answer: a password when `secret`.
    Prompt {
        text: String,
        secret: bool,
    },
    Info(String),
    Error(String),
    /// Authenticated; polkitd has been told.
    Success,
    /// Not authenticated, or the helper went away.
    Failure,
}

enum Transport {
    Socket(UnixStream),
    Child(Child),
}

/// A running helper. Dropping it ends the attempt.
pub struct Session {
    writer: Box<dyn Write>,
    transport: Transport,
}

impl Session {
    /// Start authenticating `user` for `cookie`. Events arrive on `events` from a reader
    /// thread until `Success` or `Failure`.
    pub fn start(
        helper: Option<&Path>,
        user: &str,
        cookie: &str,
        events: Sender<Event>,
    ) -> Result<Self> {
        let (reader, mut writer, transport): (Box<dyn Read + Send>, Box<dyn Write>, Transport) =
            match connect_socket() {
                Some(stream) => {
                    let mut writer = stream.try_clone().context("failed to clone the socket")?;
                    writeln!(writer, "{user}").context("failed to send the user name")?;
                    (
                        Box::new(stream.try_clone().context("failed to clone the socket")?),
                        Box::new(writer),
                        Transport::Socket(stream),
                    )
                }
                None => {
                    let path = find_helper(helper)?;
                    let mut child = Command::new(&path)
                        .arg(user)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .spawn()
                        .with_context(|| format!("failed to run {}", path.display()))?;
                    let stdin = child.stdin.take().context("no stdin for the helper")?;
                    let stdout = child.stdout.take().context("no stdout for the helper")?;
                    (Box::new(stdout), Box::new(stdin), Transport::Child(child))
                }
            };
        writeln!(writer, "{cookie}").context("failed to send the cookie")?;
        writer.flush().ok();
        thread::spawn(move || relay(reader, events));
        Ok(Self { writer, transport })
    }

    /// Answer the last prompt.
    pub fn respond(&mut self, answer: &str) -> Result<()> {
        writeln!(self.writer, "{answer}").context("failed to answer the helper")?;
        self.writer.flush().context("failed to answer the helper")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        match &mut self.transport {
            Transport::Socket(stream) => {
                let _ = stream.shutdown(Shutdown::Both);
            }
            Transport::Child(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn connect_socket() -> Option<UnixStream> {
    if !Path::new(SOCKET).exists() {
        return None;
    }
    match UnixStream::connect(SOCKET) {
        Ok(stream) => Some(stream),
        Err(err) => {
            eprintln!("warning: cannot connect to {SOCKET} ({err}); using the setuid helper");
            None
        }
    }
}

fn find_helper(configured: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = configured {
        return Ok(path.to_path_buf());
    }
    HELPER_PATHS
        .iter()
        .map(Path::new)
        .find(|path| path.is_file())
        .map(Path::to_path_buf)
        .with_context(|| {
            format!(
                "polkit-agent-helper-1 not found: {SOCKET} is missing and none of these exist: \
                 {}; set `helper:` in the typk config",
                HELPER_PATHS.join(", ")
            )
        })
}

/// Forward the helper's lines as events until it finishes or goes away.
fn relay(reader: Box<dyn Read + Send>, events: Sender<Event>) {
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { break };
        let event = parse(&line);
        let last = matches!(event, Event::Success | Event::Failure);
        if events.send_blocking(event).is_err() || last {
            return;
        }
    }
    let _ = events.send_blocking(Event::Failure);
}

fn parse(line: &str) -> Event {
    let line = unescape(line);
    if let Some(text) = line.strip_prefix("PAM_PROMPT_ECHO_OFF ") {
        Event::Prompt {
            text: text.to_string(),
            secret: true,
        }
    } else if let Some(text) = line.strip_prefix("PAM_PROMPT_ECHO_ON ") {
        Event::Prompt {
            text: text.to_string(),
            secret: false,
        }
    } else if let Some(text) = line.strip_prefix("PAM_ERROR_MSG ") {
        Event::Error(text.to_string())
    } else if let Some(text) = line.strip_prefix("PAM_TEXT_INFO ") {
        Event::Info(text.to_string())
    } else if line.starts_with("SUCCESS") {
        Event::Success
    } else {
        if !line.starts_with("FAILURE") {
            eprintln!("warning: unexpected line from polkit-agent-helper-1: {line}");
        }
        Event::Failure
    }
}

/// Undo `g_strescape`: `\n`, `\t`, `\\`, `\"` and friends, and `\ooo` octal bytes (which
/// is how non-ASCII text arrives).
fn unescape(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        i += 1;
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let Some(&next) = bytes.get(i) else { break };
        i += 1;
        match next {
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(11),
            b'0'..=b'7' => {
                let mut value = u32::from(next - b'0');
                let mut digits = 1;
                while digits < 3 {
                    match bytes.get(i) {
                        Some(d @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(d - b'0');
                            i += 1;
                            digits += 1;
                        }
                        _ => break,
                    }
                }
                out.push(value as u8);
            }
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_like_glib() {
        assert_eq!(unescape(r#"a\nb\\c\"d"#), "a\nb\\c\"d");
        assert_eq!(unescape(r"caf\303\251"), "café");
        assert_eq!(unescape(r"\1x"), "\u{1}x");
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape("trailing\\"), "trailing");
    }

    #[test]
    fn parses_helper_lines() {
        assert_eq!(
            parse("PAM_PROMPT_ECHO_OFF Password: "),
            Event::Prompt {
                text: "Password: ".into(),
                secret: true
            }
        );
        assert_eq!(
            parse("PAM_PROMPT_ECHO_ON Login: "),
            Event::Prompt {
                text: "Login: ".into(),
                secret: false
            }
        );
        assert_eq!(
            parse(r"PAM_TEXT_INFO Place your finger\non the reader"),
            Event::Info("Place your finger\non the reader".into())
        );
        assert_eq!(parse("PAM_ERROR_MSG nope"), Event::Error("nope".into()));
        assert_eq!(parse("SUCCESS"), Event::Success);
        assert_eq!(parse("FAILURE"), Event::Failure);
        assert_eq!(parse("???"), Event::Failure);
    }
}
