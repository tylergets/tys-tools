use std::path::PathBuf;

use clap::{Args, Parser};

#[derive(Debug, Parser)]
#[command(name = "tyboard", version, about = "Tyboard daemon")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Optional explicit config path (overrides discovery; useful for Nix store paths).
    #[arg(short = 'c', long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Commands {
    /// Run the clipboard manager daemon.
    Run,
    /// Print clipboard history from the cache database.
    History(HistoryArgs),
    /// Clear clipboard history from the cache database.
    #[command(name = "clear")]
    ClearHistory,
    /// Pause clipboard capture.
    #[command(name = "pause")]
    PauseCapture,
    /// Resume clipboard capture.
    #[command(name = "resume")]
    ResumeCapture,
    /// Print clipboard capture status.
    CaptureStatus,
    /// Load and validate config, then exit.
    ValidateConfig,
}

#[derive(Debug, Clone, Args)]
pub struct HistoryArgs {
    /// Emit clipboard history as JSON.
    #[arg(long)]
    pub json: bool,
    /// Include full image pixel bytes in history output.
    #[arg(long)]
    pub images: bool,
}
