//! `status [--follow] [--format plain|json|polybar]`, shared by the daemons that report
//! their state to bar modules.

use std::io::Write;

use serde::Serialize;

#[derive(Debug, Clone, clap::Args)]
pub struct StatusArgs {
    /// Keep running and print a line on every change (for bar modules).
    #[arg(short, long)]
    pub follow: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = StatusFormat::Plain)]
    pub format: StatusFormat,
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
