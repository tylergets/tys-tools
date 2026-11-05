use std::path::PathBuf;

use clap::{Args, Parser};

#[derive(Debug, Parser)]
#[command(
    name = "tyshot",
    version,
    about = "X11 screenshot tool: select, copy, save"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Optional explicit config path (overrides discovery; useful for Nix store paths).
    #[arg(short = 'c', long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Commands {
    /// Select a region on a frozen snapshot of the screen, then copy or save it (default).
    Gui(GuiArgs),
    /// Capture the whole screen without the overlay.
    Full(FullArgs),
    /// Run the tray applet.
    Tray,
    /// List recent opencode sessions screenshots can be sent to, newest first.
    Sessions,
    /// Load and validate config, then exit.
    ValidateConfig,
    /// Internal: serve a PNG from stdin on the clipboard.
    #[command(name = "serve-clipboard", hide = true)]
    ServeClipboard,
}

#[derive(Debug, Clone, Default, Args)]
pub struct GuiArgs {
    /// Wait this many seconds before taking the snapshot.
    #[arg(short, long, default_value_t = 0.0, value_name = "SECONDS")]
    pub delay: f64,
    /// Write the PNG to stdout instead of copying or saving it (any of copy/save in the
    /// overlay finishes the capture).
    #[arg(short, long)]
    pub raw: bool,
}

#[derive(Debug, Clone, Args)]
pub struct FullArgs {
    /// Wait this many seconds before taking the screenshot.
    #[arg(short, long, default_value_t = 0.0, value_name = "SECONDS")]
    pub delay: f64,
    /// Save to the output directory instead of copying to the clipboard.
    #[arg(short, long, conflicts_with = "raw")]
    pub save: bool,
    /// Write the PNG to stdout instead of copying or saving it.
    #[arg(short, long)]
    pub raw: bool,
}
