use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "tylock",
    version,
    about = "Lock the screen: your wallpaper and a clock, unlocked with your password or a finger"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Optional explicit config path (overrides discovery; useful for Nix store paths).
    #[arg(short = 'c', long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Return once the screen is locked, leaving the lock screen running in the
    /// background (for swayidle's `before-sleep`, which waits for the command).
    #[arg(short = 'f', long)]
    pub daemonize: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    /// Lock the screen until you unlock it (the default).
    Lock,
    /// Load and validate config, then exit.
    ValidateConfig,
}
