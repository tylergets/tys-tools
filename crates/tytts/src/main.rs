use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde::{Deserialize, Serialize};

mod audio;
mod engine;
mod laci;
mod native;
mod qwen;
mod sherpa;
mod voices;
mod zonos;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Engine {
    #[default]
    Pocket,
    Zonos,
    Kokoro,
    Piper,
    Qwen,
}

impl std::fmt::Display for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pocket => "pocket",
            Self::Zonos => "zonos",
            Self::Kokoro => "kokoro",
            Self::Piper => "piper",
            Self::Qwen => "qwen",
        })
    }
}

const BASE: &str = "https://huggingface.co/kyutai/pocket-tts-without-voice-cloning/resolve/d4fdd22ae8c8e1cb3634e150ebeff1dab2d16df3";

struct Asset {
    local: &'static str,
    remote: &'static str,
    bytes: u64,
    checksum: &'static str,
}

const ASSETS: &[Asset] = &[
    Asset {
        local: "model.safetensors",
        remote: "tts_b6369a24.safetensors",
        bytes: 235_738_732,
        checksum: "58aa704a88faad35f22c34ea1cb55c4c5629de8b8e035c6e4936e2673dc07617",
    },
    Asset {
        local: "tokenizer.model",
        remote: "tokenizer.model",
        bytes: 59_339,
        checksum: "d461765ae179566678c93091c5fa6f2984c31bbe990bf1aa62d92c64d91bc3f6",
    },
    Asset {
        local: "voice.safetensors",
        remote: "embeddings/alba.safetensors",
        bytes: 512_088,
        checksum: "ad234695323e4030336b6afc8a050c97e3110603e11ecd8226d9562488300a50",
    },
];

#[derive(Parser)]
#[command(
    version,
    about = "Speak locally with Pocket TTS, Zonos3-Mini, Kokoro, Piper, or Qwen3-TTS"
)]
struct Cli {
    /// Text to speak; omit or use - to read stdin.
    text: Vec<String>,
    #[arg(long)]
    config: Option<PathBuf>,
    /// Use a local bundle for the selected engine (see README).
    #[arg(long)]
    model: Option<PathBuf>,
    /// Saved/preset voice name, or a local Pocket/Zonos embedding.
    #[arg(long)]
    voice: Option<PathBuf>,
    /// Speech engine; saved voices select their engine automatically.
    #[arg(long, value_enum)]
    engine: Option<Engine>,
    /// Clone a recording into a saved voice (Zonos by default, or --engine qwen).
    #[arg(long, requires = "name", conflicts_with_all = ["text", "voice", "output", "voices", "voice_info", "remove_voice"])]
    clone: Option<PathBuf>,
    /// Name for the cloned voice.
    #[arg(long, requires = "clone")]
    name: Option<String>,
    /// Matching reference transcript for Qwen cloning (improves voice matching).
    #[arg(long, requires = "clone")]
    transcript: Option<String>,
    /// Delivery/style instruction for Qwen preset voices.
    #[arg(long)]
    instruct: Option<String>,
    /// List voices for the OS language (override with --language).
    #[arg(long, conflicts_with_all = ["text", "voice", "output", "voice_info", "remove_voice"])]
    voices: bool,
    /// List voices in every language.
    #[arg(long, requires = "voices", conflicts_with = "language")]
    all_languages: bool,
    /// Show a saved or built-in voice's details.
    #[arg(long, conflicts_with_all = ["text", "voice", "output", "remove_voice"])]
    voice_info: Option<String>,
    /// Delete a saved voice and its retained reference recording.
    #[arg(long, conflicts_with_all = ["text", "voice", "output"])]
    remove_voice: Option<String>,
    /// Speech language, or language filter for --voices.
    #[arg(long)]
    language: Option<String>,
    /// Save a WAV file instead of playing it.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Speech speed multiplier (1 = normal).
    #[arg(long)]
    speed: Option<f64>,
    /// Hide download and synthesis progress.
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    engine: Engine,
    model: Option<PathBuf>,
    voice: Option<PathBuf>,
    model_dir: Option<PathBuf>,
    auto_download: bool,
    speed: f64,
    language: Option<String>,
    gpu: bool,
    instruct: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine: Engine::Pocket,
            model: None,
            voice: None,
            model_dir: None,
            auto_download: true,
            speed: 1.0,
            language: None,
            gpu: true,
            instruct: None,
        }
    }
}

struct Progress(ProgressBar);

impl Progress {
    fn new(message: impl Into<String>, bytes: Option<u64>, quiet: bool) -> Self {
        let message = message.into();
        let bar = match bytes {
            Some(bytes) => ProgressBar::new(bytes),
            None => ProgressBar::new_spinner(),
        };
        if quiet || !io::stderr().is_terminal() {
            bar.set_draw_target(ProgressDrawTarget::hidden());
            if !quiet {
                eprintln!("{message}");
            }
        }
        let template = if bytes.is_some() {
            "{spinner:.cyan} {msg} [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec} ETA {eta}"
        } else {
            "{spinner:.cyan} {msg} [{elapsed_precise}]"
        };
        bar.set_style(ProgressStyle::with_template(template).expect("valid progress template"));
        bar.set_message(message);
        bar.enable_steady_tick(Duration::from_millis(100));
        Self(bar)
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.0.finish_and_clear();
    }
}

// Unique temporary files keep helper invocations independent.
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

fn download(dir: &Path, base: &str, asset: &Asset, enabled: bool, quiet: bool) -> Result<()> {
    let name = asset.local;
    let bytes = asset.bytes;
    let path = dir.join(name);
    if ty_common::download::is_complete(&path, bytes) {
        return Ok(());
    }
    if !enabled {
        bail!(
            "{} is missing or incomplete; enable auto_download to fetch it",
            path.display()
        );
    }
    let progress = Progress::new(format!("Downloading {name}"), Some(bytes), quiet);
    ty_common::download::fetch(
        &ty_common::download::Asset {
            url: &format!("{base}/{}", asset.remote),
            path: &path,
            bytes,
            sha256: asset.checksum,
        },
        &ty_common::download::Progress::default(),
        &std::sync::atomic::AtomicBool::new(false),
        |done| progress.0.set_position(done),
    )?;
    Ok(())
}

fn model_root(config: &Config) -> Result<PathBuf> {
    Ok(match &config.model_dir {
        Some(path) => ty_common::config::expand_user(path),
        None => dirs::data_dir()
            .context("cannot determine data directory")?
            .join("tytts/models"),
    })
}

fn model_identity(config: &Config) -> Result<String> {
    if let Some(path) = &config.model {
        return Ok(format!(
            "local:{}",
            fs::canonicalize(ty_common::config::expand_user(path))?.display()
        ));
    }
    Ok(match config.engine {
        Engine::Pocket => "pocket-tts-2026-01".into(),
        Engine::Zonos => zonos::REVISION.into(),
        Engine::Kokoro => "kokoro-multi-lang-v1_0".into(),
        Engine::Piper => "piper".into(),
        Engine::Qwen => qwen::REVISION.into(),
    })
}

fn model(config: &Config, quiet: bool) -> Result<PathBuf> {
    if let Some(path) = &config.model {
        let path = ty_common::config::expand_user(path);
        if !path.is_dir() {
            bail!("Pocket --model takes a bundle directory; use --engine piper for Piper bundles");
        }
        for name in ["model.safetensors", "tokenizer.model", "config.json"] {
            if !path.join(name).is_file() {
                bail!(
                    "local Pocket TTS bundle is missing {}",
                    path.join(name).display()
                );
            }
        }
        return Ok(path);
    }
    let dir = model_root(config)?.join("pocket-tts-2026-01");
    for asset in ASSETS {
        if asset.local == "voice.safetensors" && config.voice.is_some() {
            continue;
        }
        download(&dir, BASE, asset, config.auto_download, quiet)?;
    }
    Ok(dir)
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut config: Config = ty_common::config::load("tytts", cli.config)?.config;
    if cli.voices {
        return voices::list(
            cli.engine,
            cli.language.as_deref().or(config.language.as_deref()),
            cli.all_languages,
        );
    }
    if let Some(name) = &cli.voice_info {
        return voices::info(name);
    }
    if let Some(name) = &cli.remove_voice {
        return voices::remove(name);
    }
    if let Some(engine) = cli.engine {
        config.engine = engine;
    }
    if let Some(language) = cli.language {
        config.language = Some(language);
    }
    if let Some(model) = cli.model {
        config.model = Some(model);
    }
    if let Some(voice) = cli.voice {
        config.voice = Some(voice);
    }
    if let Some(instruct) = cli.instruct {
        config.instruct = Some(instruct);
    }
    if let Some(input) = cli.clone {
        // Cloning defaults to Zonos independently of the default speech engine.
        if cli
            .engine
            .is_some_and(|engine| !matches!(engine, Engine::Zonos | Engine::Qwen))
        {
            bail!("recording-based cloning is supported only by Zonos and Qwen");
        }
        config.engine = cli.engine.unwrap_or(Engine::Zonos);
        if config.engine != Engine::Qwen && cli.transcript.is_some() {
            bail!("--transcript is supported only for Qwen cloning");
        }
        if cli
            .transcript
            .as_deref()
            .is_some_and(|text| text.trim().is_empty())
        {
            bail!("reference transcript must not be empty");
        }
        if config.instruct.is_some() {
            bail!("style instructions apply to Qwen presets, not cloning");
        }
        let input = ty_common::config::expand_user(&input);
        if !input.is_file() {
            bail!("reference recording does not exist: {}", input.display());
        }
        let name = cli.name.as_deref().unwrap();
        voices::validate_name(name)?;
        let voice = voices::Voice {
            version: 1,
            engine: config.engine,
            model: model_identity(&config)?,
        };
        return voices::create(name, &voice, &input, |staging, reference| {
            if config.engine == Engine::Qwen {
                let dir = qwen::model(&config, true, cli.quiet)?;
                let _progress = Progress::new("Cloning voice with Qwen3-TTS", None, cli.quiet);
                return qwen::clone_voice(
                    &config,
                    &dir,
                    staging,
                    reference,
                    cli.transcript.as_deref(),
                );
            }
            let dir = zonos::model(&config, true, false, cli.quiet)?;
            let _progress = Progress::new("Cloning voice with Zonos3-Mini", None, cli.quiet);
            zonos::clone_voice(&dir, reference, &staging.join("embedding.npy"), config.gpu)
        });
    }
    let mut saved_model = None;
    if let Some(value) = config.voice.clone() {
        let name = value.to_string_lossy();
        // A plain name selects the catalog; paths retain their previous meaning.
        if !name.contains('/') && value.extension().is_none() {
            let (selected, path) = match name.as_ref() {
                "alba" => (Engine::Pocket, None),
                "savannah" => (Engine::Zonos, None),
                name if qwen::metadata(name).is_some() => (Engine::Qwen, Some(value.clone())),
                name if sherpa::voices().any(|v| v.0 == name) => {
                    let engine = sherpa::voices().find(|v| v.0 == name).unwrap().1;
                    (engine, Some(value.clone()))
                }
                name => {
                    let (voice, path) = voices::load(name)?;
                    saved_model = Some(voice.model);
                    (voice.engine, Some(path))
                }
            };
            if cli.engine.is_some_and(|e| e != selected) {
                bail!("voice belongs to {selected}, conflicting with --engine");
            }
            config.engine = selected;
            config.voice = path;
        }
    }
    if let Some(expected) = saved_model {
        if expected != model_identity(&config)? {
            bail!("voice was created for model {expected}; select the matching --model");
        }
    }
    if config.engine == Engine::Pocket {
        if config
            .language
            .as_deref()
            .is_some_and(|l| !matches!(l, "en" | "en_us"))
        {
            bail!("the bundled Pocket model supports English only");
        }
    }
    if matches!(config.engine, Engine::Kokoro | Engine::Piper) {
        sherpa::validate(&config)?;
    }
    if config.engine != Engine::Qwen && config.instruct.is_some() {
        bail!("style instructions require --engine qwen or a Qwen preset");
    }
    if let Some(speed) = cli.speed {
        config.speed = speed;
    }
    if !config.speed.is_finite() || !(0.25..=4.0).contains(&config.speed) {
        bail!("speed must be between 0.25 and 4");
    }
    let text = if cli.text.is_empty() || cli.text == ["-"] {
        if cli.text.is_empty() && io::stdin().is_terminal() {
            bail!("provide text: tytts \"Hello\", or pipe text into tytts");
        }
        let mut text = String::new();
        let _progress = Progress::new("Reading stdin (waiting for EOF)", None, cli.quiet);
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
    let dir = match config.engine {
        Engine::Pocket => model(&config, cli.quiet)?,
        Engine::Zonos => zonos::model(&config, false, config.voice.is_none(), cli.quiet)?,
        Engine::Kokoro | Engine::Piper => sherpa::model(&config, cli.quiet)?,
        Engine::Qwen => qwen::prepare(&config, cli.quiet)?,
    };
    let voice = config.voice.as_ref().map_or_else(
        || match config.engine {
            Engine::Pocket => dir.join("voice.safetensors"),
            Engine::Zonos => dir.join("savannah.npy"),
            Engine::Kokoro | Engine::Piper => PathBuf::from(sherpa::default_voice(config.engine)),
            Engine::Qwen => PathBuf::from("qwen-aiden"),
        },
        |path| ty_common::config::expand_user(path),
    );
    if matches!(config.engine, Engine::Pocket | Engine::Zonos) && !voice.is_file() {
        bail!("voice embedding is missing: {}", voice.display());
    }
    let loading = Progress::new(
        format!("Loading {} model and voice", config.engine),
        None,
        cli.quiet,
    );
    let mut engine = engine::Loaded::load(&config, &dir, &voice)?;
    drop(loading);
    let progress = Progress::new("Generating speech", None, cli.quiet);
    let start = std::time::Instant::now();
    let (sample_rate, chunks) =
        engine.stream(&text, &voice, config.language.as_deref().unwrap_or("en_us"))?;
    // Zonos may generate much slower than real time on desktop CPUs. Spool to
    // disk before playback so generation gaps never become gaps in speech.
    let buffered = if config.engine == Engine::Zonos && cli.output.is_none() {
        Some(Temporary::create(&std::env::temp_dir(), "speech.wav")?.0)
    } else {
        None
    };
    let output_path = cli
        .output
        .as_deref()
        .or_else(|| buffered.as_ref().map(|p| p.0.as_path()));
    let mut output = audio::Output::new(output_path, sample_rate)?;
    let mut tempo = if config.speed != 1.0 {
        Some(audio::Tempo::new(sample_rate, config.speed)?)
    } else {
        None
    };
    let mut generated = 0usize;
    // Consume PCM in bounded blocks, including completed native-runtime WAVs.
    for chunk in chunks {
        let mut samples = chunk.context("generating speech")?;
        if generated == 0 && !samples.is_empty() && !cli.quiet {
            progress.0.println(format!(
                "First audio ready in {:.2}s",
                start.elapsed().as_secs_f64()
            ));
        }
        generated += samples.len();
        if let Some(tempo) = &mut tempo {
            tempo.write(&mut samples)?;
            samples = tempo.read();
        }
        output.write(&samples)?;
        let action = if buffered.is_some() {
            "Generating"
        } else if cli.output.is_some() {
            "Saving"
        } else {
            "Speaking"
        };
        progress.0.set_message(format!(
            "{action} · {:.1}s audio generated",
            generated as f64 / sample_rate as f64
        ));
    }
    if generated == 0 {
        bail!("{} generated no audio", config.engine);
    }
    engine.check()?;
    if let Some(tempo) = &mut tempo {
        tempo.flush()?;
        output.write(&tempo.read())?;
    }
    progress.0.set_message(if output_path.is_some() {
        "Finalizing WAV"
    } else {
        "Finishing playback"
    });
    output.finish()?;
    // Release the model/helper before playing the completed utterance.
    drop(engine);
    if let Some(buffered) = &buffered {
        progress.0.set_message("Speaking");
        audio::play_wav(&buffered.0)?;
    }
    drop(progress);
    if !cli.quiet {
        eprintln!(
            "Done · {:.1}s audio generated",
            generated as f64 / sample_rate as f64
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    run()
}
