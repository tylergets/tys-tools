use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ty_common::status::StatusArgs;

#[derive(Debug, Parser)]
#[command(
    name = "tylight",
    version,
    about = "Warmer screen colors at night (X11 and wlroots), with monitor brightness control"
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
    /// Run the daemon (and tray icon).
    Run,
    /// Turn color adjustment back on.
    Enable,
    /// Turn color adjustment off (fades back to neutral).
    Disable {
        /// Only for this long, e.g. 30m, 1h, 1h30m.
        #[arg(
            long = "for",
            value_name = "DURATION",
            conflicts_with = "until_morning"
        )]
        duration: Option<String>,
        /// Until the day setting starts again.
        #[arg(long)]
        until_morning: bool,
    },
    /// Disable if enabled, otherwise enable.
    Toggle,
    /// Hold a fixed color temperature (in kelvin) until `auto`.
    Set { kelvin: u32 },
    /// Follow the sun again after `set`.
    Auto,
    /// Apply a preset from the config (until `auto`).
    Preset { name: String },
    /// List the presets.
    Presets,
    /// Turn brightness from the ambient light sensor on, off or toggle it.
    AutoBrightness {
        #[arg(value_parser = ["on", "off", "toggle"])]
        state: Option<String>,
    },
    /// Open the graph window of temperature and brightness (needs the tray).
    Graph,
    /// Show or change the monitors' hardware brightness: 60, +10, -10.
    Brightness {
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,
    },
    /// Print the daemon's status.
    Status(StatusArgs),
    /// Print the location tylight would use (config, or a fresh GeoClue fix).
    Location,
    /// Load and validate config, then exit.
    ValidateConfig,
}
