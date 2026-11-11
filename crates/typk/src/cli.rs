use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "typk",
    version,
    about = "Polkit authentication agent: asks for your password (or fingerprint) when an action needs it"
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
    /// Run the agent: register with polkit and show a prompt when asked.
    Run,
    /// Ask you to confirm it's you (password or finger, through the running agent) and
    /// exit 0 if you did. Scripts and AI agents can gate an action on this without ever
    /// handling the password. Exit 1 when refused or cancelled, 2 on error.
    Verify {
        /// Why confirmation is asked, shown in the prompt.
        #[arg(long)]
        reason: Option<String>,
        /// Who is asking, shown as the prompt's title (with its icon when it has a desktop
        /// entry). Default: the program that ran typk, skipping shells.
        #[arg(long)]
        app: Option<String>,
        /// The polkit action to ask for. The default is typk's own (auth_self); if polkit
        /// doesn't know it, org.freedesktop.policykit.exec (admin) is asked for instead.
        #[arg(long, default_value = crate::polkit::VERIFY_ACTION)]
        action: String,
    },
    /// Load and validate config, then exit.
    ValidateConfig,
}
