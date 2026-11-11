//! How `typk verify` tells the prompt who asks and why. polkitd lets only root pass
//! details through to the agent, so `verify` sends one tab-separated line to the agent's
//! instance socket beforehand (`verify <pid> <app pid> <app name> <reason>`), and the
//! agent matches it to the request by the subject pid polkitd reports. Nothing happens
//! without a running agent.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

use crate::appinfo::App;
use crate::config::APP_NAME;

const IO_TIMEOUT: Duration = Duration::from_millis(300);
/// Announcements nobody asked about are forgotten after this.
const TTL: Duration = Duration::from_secs(120);
/// Longest text shown.
const MAX_CHARS: usize = 400;

/// What `typk verify` said about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Announcement {
    pub app: Option<App>,
    pub reason: Option<String>,
}

/// Tell the running agent who is about to ask for authorization (the program that ran
/// typk, unless `app` names it) and why.
pub fn announce(reason: Option<&str>, app: Option<&str>) {
    let path = ty_common::lock::socket_path(APP_NAME);
    let Ok(mut stream) = UnixStream::connect(&path) else {
        return;
    };
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let parent = crate::appinfo::parent_pid(std::process::id()).unwrap_or(0);
    let found = App::of_process(parent);
    let app_pid = found.as_ref().map_or(parent, |found| found.pid);
    let app_name = app
        .map(str::to_string)
        .or(found.map(|found| found.name))
        .unwrap_or_default();
    let _ = writeln!(
        stream,
        "verify\t{}\t{app_pid}\t{}\t{}",
        std::process::id(),
        clean(&app_name),
        clean(reason.unwrap_or(""))
    );
}

/// One line's worth, with no tabs.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\n' | '\t' | '\r'))
        .take(MAX_CHARS)
        .collect()
}

/// The announcements made to the agent, by pid.
#[derive(Default)]
pub struct Reasons {
    by_pid: HashMap<u32, (Instant, Announcement)>,
}

impl Reasons {
    /// Read whatever clients are waiting on `listener` (which must be non-blocking).
    pub fn accept(&mut self, listener: &UnixListener) {
        while let Ok((stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
            let mut text = String::new();
            let _ = (&stream).take(4096).read_to_string(&mut text);
            for line in text.lines() {
                if let Some((pid, announcement)) = parse(line) {
                    self.by_pid.insert(pid, (Instant::now(), announcement));
                }
            }
        }
        let now = Instant::now();
        self.by_pid
            .retain(|_, (at, _)| now.duration_since(*at) < TTL);
    }

    /// What `pid` announced, if anything (handed out once).
    pub fn take(&mut self, pid: u32) -> Option<Announcement> {
        self.by_pid
            .remove(&pid)
            .map(|(_, announcement)| announcement)
    }
}

fn parse(line: &str) -> Option<(u32, Announcement)> {
    let mut fields = line.split('\t');
    if fields.next()? != "verify" {
        return None;
    }
    let pid: u32 = fields.next()?.parse().ok()?;
    let app_pid: u32 = fields.next()?.parse().ok()?;
    let app_name = fields.next()?.trim();
    let reason = fields.next().unwrap_or("").trim();
    let app = (!app_name.is_empty() && app_pid > 1).then(|| App {
        pid: app_pid,
        name: app_name.chars().take(MAX_CHARS).collect(),
    });
    let reason = (!reason.is_empty()).then(|| reason.chars().take(MAX_CHARS).collect());
    Some((pid, Announcement { app, reason }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lines() {
        let (pid, announcement) = parse("verify\t42\t41\tClaude\tDeploy to prod?").unwrap();
        assert_eq!(pid, 42);
        assert_eq!(
            announcement.app,
            Some(App {
                pid: 41,
                name: "Claude".into()
            })
        );
        assert_eq!(announcement.reason.as_deref(), Some("Deploy to prod?"));
        let (_, bare) = parse("verify\t42\t0\t\t").unwrap();
        assert_eq!(bare, Announcement::default());
        assert_eq!(parse("verify\tx\t1\ta\tb"), None);
        assert_eq!(parse("reason 42 hi"), None);
    }

    #[test]
    fn round_trips_through_a_socket() {
        let path = std::env::temp_dir().join(format!("typk-reasons-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut client = UnixStream::connect(&path).unwrap();
        writeln!(client, "verify\t7\t6\tThing\tBecause").unwrap();
        drop(client);
        let mut reasons = Reasons::default();
        reasons.accept(&listener);
        let announcement = reasons.take(7).unwrap();
        assert_eq!(announcement.reason.as_deref(), Some("Because"));
        assert_eq!(announcement.app.unwrap().name, "Thing");
        assert!(reasons.take(7).is_none());
        std::fs::remove_file(path).unwrap();
    }
}
