use std::path::PathBuf;

use clap::{Args, Parser};
use ty_common::status::StatusArgs;
use tycapture::capture::Target;

#[derive(Debug, Parser)]
#[command(name = "tycapture", version, about = "X11 screen capture applet")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Optional explicit config path (overrides discovery; useful for Nix store paths).
    #[arg(short = 'c', long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Commands {
    /// Run the tray applet.
    Run,
    /// Start a recording in the running applet.
    Start(StartArgs),
    /// Stop the current recording (or cancel a countdown).
    Stop,
    /// Stop if recording, otherwise start.
    Toggle(StartArgs),
    /// Print the applet's status.
    Status(StatusArgs),
    /// Load and validate config, then exit.
    ValidateConfig,
}

#[derive(Debug, Clone, Args)]
pub struct StartArgs {
    /// What to record.
    #[arg(value_enum, default_value_t = Target::Screen)]
    pub target: Target,
    /// Skip the setup dialog and reuse the last-used options.
    #[arg(long)]
    pub now: bool,
}
