use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ty_common::status::StatusArgs;

#[derive(Debug, Parser)]
#[command(
    name = "tyci",
    version,
    about = "Open GitHub PRs and running CI in your tray, through the gh CLI"
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
    /// Fetch once and print running CI, recently finished runs and open PRs.
    List {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Print counts of running and failed runs and open PRs (for bar modules).
    Status(StatusArgs),
    /// Load and validate the config, then exit.
    ValidateConfig,
}
