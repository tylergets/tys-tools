use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ty_common::status::StatusArgs;

#[derive(Debug, Parser)]
#[command(name = "tykey", version, about = "Minimal text expansion CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Optional explicit config path (overrides discovery; useful for Nix store paths).
    #[arg(short = 'c', long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Enable debug logging for trigger matching internals.
    #[arg(long, global = true)]
    pub debug: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    /// Run key listener and trigger expansion output.
    Run,
    /// Load and validate config, then exit.
    ValidateConfig,
    /// Print the daemon's status (for bar modules).
    Status(StatusArgs),
}
