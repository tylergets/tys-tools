use std::path::PathBuf;

use clap::Parser;
use ty_common::status::StatusArgs;

#[derive(Debug, Parser)]
#[command(
    name = "tyspeak",
    version,
    about = "Speech-to-text dictation tray daemon with a D-Bus API"
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
    /// Run the tray daemon.
    Run,
    /// Start dictating.
    Start,
    /// Stop dictating and type/copy the text.
    Stop,
    /// Stop if dictating, otherwise start.
    Toggle,
    /// Stop dictating and discard the audio.
    Cancel,
    /// Print the daemon's status.
    Status(StatusArgs),
    /// List the available models.
    Models,
    /// Select the model to use (downloaded first if needed).
    Model {
        /// Model id, as listed by `tyspeak models`.
        id: String,
    },
    /// Transcribe a WAV file without the daemon (downloads the model if needed).
    Transcribe {
        file: PathBuf,
        /// Model id; defaults to the configured model.
        #[arg(short, long)]
        model: Option<String>,
    },
    /// Load and validate config, then exit.
    ValidateConfig,
}
