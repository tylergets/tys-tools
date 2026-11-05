use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ty_common::status::StatusArgs;

#[derive(Debug, Parser)]
#[command(
    name = "typort",
    version,
    about = "Your listening ports (dev servers, ...): open them in the browser or stop them"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Optional explicit config path (overrides discovery; useful for Nix store paths).
    #[arg(short = 'c', long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    /// Run the tray applet.
    Run,
    /// List your listening ports.
    #[command(alias = "ls")]
    List {
        /// Print JSON.
        #[arg(long)]
        json: bool,
        /// Include ports hidden by the config.
        #[arg(short, long)]
        all: bool,
    },
    /// Open a port in the browser.
    Open { port: u16 },
    /// Stop whatever listens on a port (SIGTERM).
    Kill {
        port: u16,
        /// Use SIGKILL.
        #[arg(short, long)]
        force: bool,
    },
    /// Print the URL of a port.
    Url { port: u16 },
    /// Print the open ports, for bars.
    Status(StatusArgs),
    /// Load and validate config, then exit.
    ValidateConfig,
}
