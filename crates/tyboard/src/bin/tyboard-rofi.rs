use std::io::Write;
use std::process::{Command, ExitStatus, Stdio};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use tyboard::clipboard::backend::{menu_label, set_clipboard_value};
use tyboard::clipboard::{ClipboardEntry, SharedClipboardState, DEFAULT_HISTORY_LIMIT};

const DEFAULT_PROMPT: &str = "tyboard";
const DEFAULT_ROFI_BIN: &str = "rofi";
const DEFAULT_LINES: usize = 15;
const MENU_LABEL_CHAR_LIMIT: usize = 120;

#[derive(Debug, Parser)]
#[command(
    name = "tyboard-rofi",
    version,
    about = "Pick clipboard history via rofi"
)]
struct Cli {
    /// Prompt shown in rofi.
    #[arg(long, default_value = DEFAULT_PROMPT)]
    prompt: String,

    /// Number of menu rows rofi should display.
    #[arg(long, default_value_t = DEFAULT_LINES)]
    lines: usize,

    /// rofi executable to invoke.
    #[arg(long, default_value = DEFAULT_ROFI_BIN)]
    rofi_bin: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let shared_state = SharedClipboardState::load_default(DEFAULT_HISTORY_LIMIT)?;
    let entries = shared_state.history_snapshot();

    if entries.is_empty() {
        return Ok(());
    }

    let selected = prompt_selection(&cli, &entries)?;
    let Some(index) = selected else {
        return Ok(());
    };
    let entry = entries
        .get(index)
        .ok_or_else(|| anyhow!("selected entry index out of range: {index}"))?;

    gtk::init().context("failed to initialize GTK for clipboard access")?;
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    set_clipboard_value(&clipboard, entry);
    Ok(())
}

fn prompt_selection(cli: &Cli, entries: &[ClipboardEntry]) -> Result<Option<usize>> {
    let mut child = Command::new(&cli.rofi_bin)
        .arg("-dmenu")
        .arg("-i")
        .arg("-p")
        .arg(&cli.prompt)
        .arg("-lines")
        .arg(cli.lines.to_string())
        .arg("-format")
        .arg("i")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to launch {}", cli.rofi_bin))?;

    let menu_input = entries
        .iter()
        .map(|entry| menu_label(entry, MENU_LABEL_CHAR_LIMIT))
        .collect::<Vec<_>>()
        .join("\n");

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("failed to open stdin for rofi"))?;
        stdin
            .write_all(menu_input.as_bytes())
            .context("failed writing menu entries to rofi stdin")?;
    }

    let output = child
        .wait_with_output()
        .context("failed waiting for rofi selection")?;

    if is_rofi_cancel(&output.status) {
        return Ok(None);
    }
    if !output.status.success() {
        return Err(anyhow!(
            "rofi exited with non-zero status: {}",
            output.status
        ));
    }

    let trimmed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let index = trimmed
        .parse::<usize>()
        .with_context(|| format!("failed to parse rofi selection index: {trimmed}"))?;
    Ok(Some(index))
}

fn is_rofi_cancel(status: &ExitStatus) -> bool {
    status.code() == Some(1)
}
