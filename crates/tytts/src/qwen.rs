use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::{native, Asset, Config, Engine, Temporary};

pub const REVISION: &str = "qwen3-tts-1.7b-b7ee2e8c";
const BASE: &str = "https://huggingface.co/Serveurperso/Qwen3-TTS-GGUF/resolve/b7ee2e8c7459c3bea99da23e3d178125a7d1713c";
const CUSTOM: Asset = Asset {
    local: "qwen-talker-1.7b-customvoice-BF16.gguf",
    remote: "qwen-talker-1.7b-customvoice-BF16.gguf",
    bytes: 3839585664,
    checksum: "dd5d70d3f7d09ffc707f4c25eabb1773e0f1f568e02c17ca72a08592334c276c",
};
const CLONE: Asset = Asset {
    local: "qwen-talker-1.7b-base-BF16.gguf",
    remote: "qwen-talker-1.7b-base-BF16.gguf",
    bytes: 3876199616,
    checksum: "7f4567113f3aa07e842fac6d17a10a98cf3d0068c0871ac245ac9befc63affa5",
};
const CODEC: Asset = Asset {
    local: "qwen-tokenizer-12hz-F32.gguf",
    remote: "qwen-tokenizer-12hz-F32.gguf",
    bytes: 647263104,
    checksum: "b16b95557c7c7340a121757bd6855b9609e1cf4ad3fad0778b89393293ae5f3d",
};

// Prefix names to keep the shared preset namespace unambiguous.
const VOICES: &[(&str, &str, &str)] = &[
    ("qwen-aiden", "en-us", "male"),
    ("qwen-ryan", "en", "male"),
    ("qwen-vivian", "zh", "female"),
    ("qwen-serena", "zh", "female"),
    ("qwen-uncle-fu", "zh", "male"),
    ("qwen-dylan", "zh", "male"),
    ("qwen-eric", "zh", "male"),
    ("qwen-ono-anna", "ja", "female"),
    ("qwen-sohee", "ko", "female"),
];

pub fn voices() -> impl Iterator<Item = (&'static str, Engine)> {
    VOICES.iter().map(|v| (v.0, Engine::Qwen))
}

pub fn metadata(name: &str) -> Option<(&'static str, &'static str)> {
    VOICES.iter().find(|v| v.0 == name).map(|v| (v.1, v.2))
}

fn language(value: &str) -> Result<&'static str> {
    let value = value.trim().to_lowercase().replace('_', "-");
    Ok(match value.split(['-', '.', '@']).next().unwrap_or("") {
        "auto" => "auto",
        "en" | "english" => "English",
        "zh" | "cmn" | "chinese" => "Chinese",
        "ja" | "japanese" => "Japanese",
        "ko" | "korean" => "Korean",
        "de" | "german" => "German",
        "fr" | "french" => "French",
        "ru" | "russian" => "Russian",
        "pt" | "portuguese" => "Portuguese",
        "es" | "spanish" => "Spanish",
        "it" | "italian" => "Italian",
        _ => bail!("Qwen supports en, zh, ja, ko, de, fr, ru, pt, es, it, or auto"),
    })
}

fn is_clone(config: &Config) -> bool {
    config
        .voice
        .as_ref()
        .is_some_and(|p| p.extension().is_some_and(|e| e == "spk"))
}

pub fn validate(config: &Config) -> Result<()> {
    if let Some(lang) = &config.language {
        language(lang)?;
    }
    if let Some(voice) = &config.voice {
        if is_clone(config) && !ty_common::config::expand_user(voice).is_file() {
            bail!("Qwen speaker embedding is missing: {}", voice.display());
        }
        if !is_clone(config) && metadata(&voice.to_string_lossy()).is_none() {
            bail!("unknown Qwen voice; use --voices --engine qwen, or a saved Qwen clone");
        }
    }
    if is_clone(config) && config.instruct.is_some() {
        bail!("Qwen style instructions require a preset voice, not a clone");
    }
    Ok(())
}

pub fn model(config: &Config, clone: bool, quiet: bool) -> Result<PathBuf> {
    let talker = if clone { &CLONE } else { &CUSTOM };
    let dir = config
        .model
        .as_ref()
        .map(|p| ty_common::config::expand_user(p))
        .unwrap_or(crate::model_root(config)?.join(REVISION));
    for asset in [talker, &CODEC] {
        if config.model.is_some() {
            if !dir.join(asset.local).is_file() {
                bail!(
                    "custom Qwen bundle is missing {}",
                    dir.join(asset.local).display()
                );
            }
        } else {
            crate::download(&dir, BASE, asset, config.auto_download, quiet)?;
        }
    }
    Ok(dir)
}

pub fn prepare(config: &Config, quiet: bool) -> Result<PathBuf> {
    validate(config)?;
    model(config, is_clone(config), quiet)
}

pub fn clone_voice(
    config: &Config,
    dir: &Path,
    staging: &Path,
    reference: &Path,
    transcript: Option<&str>,
) -> Result<()> {
    // Normalize any FFmpeg-readable recording; keep the original alongside it.
    let wav = staging.join("clone.wav");
    native::run(
        native::command("ffmpeg")
            .args(["-nostdin", "-v", "error", "-y", "-i"])
            .arg(reference)
            .args(["-ac", "1", "-ar", "24000", "-c:a", "pcm_s16le"])
            .arg(&wav),
    )?;
    let mut cmd = native::command("qwen-codec");
    cmd.arg("--model")
        .arg(dir.join(CODEC.local))
        .arg("--talker")
        .arg(dir.join(CLONE.local))
        .arg("-i")
        .arg(&wav);
    if !config.gpu {
        cmd.args(["--device", "CPU"]);
    }
    native::run(&mut cmd)?;
    for file in ["clone.spk", "clone.rvq"] {
        if fs::metadata(staging.join(file))?.len() == 0 {
            bail!("Qwen produced empty {file}");
        }
    }
    if let Some(text) = transcript {
        if text.trim().is_empty() {
            bail!("reference transcript must not be empty");
        }
        fs::write(staging.join("clone.txt"), text)?;
    }
    Ok(())
}

pub struct Synth {
    dir: PathBuf,
    voice: PathBuf,
    language: &'static str,
    instruct: Option<String>,
    gpu: bool,
    clone: bool,
}

impl Synth {
    pub fn new(config: &Config, dir: &Path) -> Result<Self> {
        validate(config)?;
        let voice = config
            .voice
            .as_ref()
            .map(|p| ty_common::config::expand_user(p))
            .unwrap_or_else(|| "qwen-aiden".into());
        let default_language = metadata(&voice.to_string_lossy()).map_or("en", |v| v.0);
        Ok(Self {
            dir: dir.to_owned(),
            voice,
            language: language(config.language.as_deref().unwrap_or(default_language))?,
            instruct: config.instruct.clone(),
            gpu: config.gpu,
            clone: is_clone(config),
        })
    }

    pub fn stream(&self, text: &str) -> Result<(u32, crate::engine::AudioStream)> {
        let (wav, file) = Temporary::create(&std::env::temp_dir(), "qwen.wav")?;
        drop(file);
        let mut cmd = native::command("qwen-tts");
        cmd.arg("--model")
            .arg(self.dir.join(if self.clone {
                CLONE.local
            } else {
                CUSTOM.local
            }))
            .arg("--codec")
            .arg(self.dir.join(CODEC.local))
            .args(["--lang", self.language, "--format", "wav16"])
            .arg("-o")
            .arg(&wav.0);
        if !self.gpu {
            cmd.args(["--device", "CPU"]);
        }
        if self.clone {
            cmd.arg("--ref-spk").arg(&self.voice);
            let transcript = self.voice.with_extension("txt");
            if transcript.is_file() {
                cmd.arg("--ref-text")
                    .arg(transcript)
                    .arg("--ref-rvq")
                    .arg(self.voice.with_extension("rvq"));
            }
        } else {
            let name = self.voice.to_str().context("Qwen voice must be UTF-8")?;
            cmd.arg("--speaker")
                .arg(name.trim_start_matches("qwen-").replace('-', "_"));
            if let Some(instruct) = &self.instruct {
                cmd.arg("--instruct").arg(instruct);
            }
        }
        native::run_text(&mut cmd, text)?;
        native::wav_stream(wav)
    }
}
