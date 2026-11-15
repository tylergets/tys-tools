use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const VOICE: &str = "en_US-lessac-medium.onnx";
const BASE: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/e9ab671f2a0e50a701b34361136ba3e6a3a9eb0c/en/en_US/lessac/medium";

#[derive(Parser)]
#[command(
    version,
    about = "Speak text locally; downloads the default voice on first use"
)]
struct Cli {
    /// Text to speak; omit or use - to read stdin.
    text: Vec<String>,
    #[arg(long)]
    config: Option<PathBuf>,
    /// Use a local Piper ONNX model (with an adjacent .onnx.json file).
    #[arg(long)]
    model: Option<PathBuf>,
    /// Save a WAV file instead of playing it.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Speech speed multiplier (1 = normal).
    #[arg(long)]
    speed: Option<f64>,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    model: Option<PathBuf>,
    model_dir: Option<PathBuf>,
    auto_download: bool,
    speed: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: None,
            model_dir: None,
            auto_download: true,
            speed: 1.0,
        }
    }
}

// Unique files keep simultaneous invocations independent. Rename only verified downloads.
struct Temporary(PathBuf);

impl Temporary {
    fn create(dir: &Path, name: &str) -> Result<(Self, File)> {
        for suffix in 0..1000 {
            let path = dir.join(format!(".tytts-{}-{suffix}-{name}", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Self(path), file)),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err).context("creating temporary file"),
            }
        }
        bail!("unable to create a temporary file in {}", dir.display())
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn download(dir: &Path, name: &str, bytes: u64, checksum: &str, enabled: bool) -> Result<()> {
    let path = dir.join(name);
    if fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == bytes) {
        return Ok(());
    }
    if !enabled {
        bail!(
            "{} is missing or incomplete; enable auto_download to fetch it",
            path.display()
        );
    }
    fs::create_dir_all(dir)?;
    eprintln!("Downloading {name} ({:.1} MB)…", bytes as f64 / 1_000_000.0);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(1800)))
        .build()
        .into();
    let mut response = agent
        .get(format!("{BASE}/{name}"))
        .call()
        .with_context(|| format!("downloading {name}; run tytts again to retry"))?;
    let (partial, mut file) = Temporary::create(dir, name)?;
    let mut reader = response.body_mut().as_reader();
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut buffer = [0u8; 65536];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > bytes {
            bail!("download of {name} exceeded its expected size");
        }
        file.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
    }
    if total != bytes || format!("{:x}", hash.finalize()) != checksum {
        bail!("download of {name} failed checksum/size verification; run tytts again to retry");
    }
    file.sync_all()?;
    fs::rename(&partial.0, path)?;
    Ok(())
}

fn model(config: &Config) -> Result<PathBuf> {
    if let Some(path) = &config.model {
        let path = ty_common::config::expand_user(path);
        let mut metadata = path.as_os_str().to_os_string();
        metadata.push(".json");
        if !path.is_file() || !Path::new(&metadata).is_file() {
            bail!(
                "local model needs both {} and its adjacent .json file",
                path.display()
            );
        }
        return Ok(path);
    }
    let dir = match &config.model_dir {
        Some(path) => ty_common::config::expand_user(path),
        None => dirs::data_dir()
            .context("cannot determine data directory")?
            .join("tytts/models"),
    };
    download(
        &dir,
        VOICE,
        63_201_294,
        "5efe09e69902187827af646e1a6e9d269dee769f9877d17b16b1b46eeaaf019f",
        config.auto_download,
    )?;
    download(
        &dir,
        &format!("{VOICE}.json"),
        4885,
        "efe19c417bed055f2d69908248c6ba650fa135bc868b0e6abb3da181dab690a0",
        config.auto_download,
    )?;
    Ok(dir.join(VOICE))
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut config: Config = ty_common::config::load("tytts", cli.config)?.config;
    if let Some(model) = cli.model {
        config.model = Some(model);
    }
    if let Some(speed) = cli.speed {
        config.speed = speed;
    }
    if !config.speed.is_finite() || config.speed <= 0.0 || !(1.0 / config.speed).is_finite() {
        bail!("speed must be a finite number greater than zero");
    }
    let text = if cli.text.is_empty() || cli.text == ["-"] {
        if cli.text.is_empty() && io::stdin().is_terminal() {
            bail!("provide text: tytts \"Hello\", or pipe text into tytts");
        }
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .context("reading stdin")?;
        text
    } else {
        cli.text.join(" ")
    };
    if text.trim().is_empty() {
        bail!("no text to speak");
    }
    let model = model(&config)?;
    let temporary = if cli.output.is_none() {
        Some(Temporary::create(&std::env::temp_dir(), "speech.wav")?.0)
    } else {
        None
    };
    let output = cli
        .output
        .as_deref()
        .unwrap_or_else(|| &temporary.as_ref().unwrap().0);
    let mut child = Command::new("piper")
        .arg("--model")
        .arg(model)
        .arg("--length_scale")
        .arg((1.0 / config.speed).to_string())
        .arg("--output_file")
        .arg(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .context("starting piper; install piper-tts or use the Nix package")?;
    // Close stdin even on a failed write, then reap the child before reporting errors.
    // Piper versions differ in handling multiple lines with one output filename.
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let input_result = child
        .stdin
        .take()
        .context("opening Piper stdin")?
        .write_all(format!("{text}\n").as_bytes());
    let status = child.wait().context("waiting for Piper")?;
    if !status.success() {
        bail!("Piper failed: {status}");
    }
    input_result.context("sending text to Piper")?;
    if cli.output.is_none() {
        let status = Command::new("paplay")
            .arg(output)
            .status()
            .context("starting paplay; install pulseaudio or use the Nix package")?;
        if !status.success() {
            bail!("audio playback failed: {status}");
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    run()
}
