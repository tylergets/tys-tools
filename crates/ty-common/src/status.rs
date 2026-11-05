//! `status [--follow] [--format plain|json|polybar]`, shared by the daemons that report
//! their state to bar modules.

use std::io::Write;
use std::time::Duration;

use clap::builder::ArgPredicate;
use serde::de::DeserializeOwned;
use serde::Serialize;

#[derive(Debug, Clone, clap::Args)]
pub struct StatusArgs {
    /// Keep running and print a line on every change (for bar modules).
    #[arg(short, long)]
    pub follow: bool,
    /// Output format.
    #[arg(
        long,
        value_enum,
        default_value_t = StatusFormat::Plain,
        default_value_if("json", ArgPredicate::IsPresent, "json")
    )]
    pub format: StatusFormat,
    /// Same as `--format json`.
    #[arg(long, conflicts_with = "format")]
    pub json: bool,
}

impl StatusArgs {
    /// Print `fetch()` once, or with `--follow` check every `every` and print each change.
    pub fn run<R: Report>(&self, every: Duration, mut fetch: impl FnMut() -> R) {
        if !self.follow {
            fetch().print(self.format);
            return;
        }
        let mut last = None;
        loop {
            let line = fetch().format(self.format);
            if last.as_ref() != Some(&line) {
                let mut stdout = std::io::stdout();
                let _ = writeln!(stdout, "{line}");
                let _ = stdout.flush();
                last = Some(line);
            }
            std::thread::sleep(every);
        }
    }
}

/// A status as the JSON line a daemon sends back for `status` (see [`crate::lock::serve`]).
pub fn to_reply(status: &impl Serialize) -> String {
    serde_json::to_string(status).unwrap_or_default()
}

/// Ask the running `app` for its status; `None` when it isn't running or answers oddly.
pub fn request<T: DeserializeOwned>(app: &str) -> Option<T> {
    let reply = crate::lock::request(app, "status").ok()?;
    serde_json::from_str(&reply).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum StatusFormat {
    Plain,
    Json,
    /// Polybar color tags; prints an empty line when idle.
    Polybar,
}

/// A daemon status that can be printed in every [`StatusFormat`].
pub trait Report: Serialize {
    fn format_plain(&self) -> String;
    fn format_polybar(&self) -> String;

    fn format(&self, format: StatusFormat) -> String {
        match format {
            StatusFormat::Plain => self.format_plain(),
            StatusFormat::Json => serde_json::to_string(self).unwrap_or_default(),
            StatusFormat::Polybar => self.format_polybar(),
        }
    }

    /// Print one line and flush (bar modules read line by line).
    fn print(&self, format: StatusFormat) {
        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{}", self.format(format));
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        status: StatusArgs,
    }

    #[test]
    fn json_flag_is_format_json() {
        let parse =
            |args: &[&str]| Cli::try_parse_from([&["x"], args].concat()).map(|c| c.status.format);
        assert_eq!(parse(&[]).unwrap(), StatusFormat::Plain);
        assert_eq!(parse(&["--json"]).unwrap(), StatusFormat::Json);
        assert_eq!(
            parse(&["--format", "polybar"]).unwrap(),
            StatusFormat::Polybar
        );
        assert!(parse(&["--json", "--format", "plain"]).is_err());
    }
}
