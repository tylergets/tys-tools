use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::{native, Asset, Config, Engine, Temporary};

const BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models";
const KOKORO: Asset = Asset {
    // The int8 export introduces narrow-band ringing in the synthesized audio.
    local: "kokoro-multi-lang-v1_0.tar.bz2",
    remote: "kokoro-multi-lang-v1_0.tar.bz2",
    bytes: 349886476,
    checksum: "60703707bc05ccd48d60e1eb723f38b07d6e474795a0b6a588d7220400e0f7a4",
};
const LESSAC: Asset = Asset {
    local: "vits-piper-en_US-lessac-medium.tar.bz2",
    remote: "vits-piper-en_US-lessac-medium.tar.bz2",
    bytes: 67230653,
    checksum: "9e3febfacf0abf4270172d2958bcec246032b7e88efc2720840cc80c93de334e",
};
const AMY: Asset = Asset {
    local: "vits-piper-en_US-amy-low.tar.bz2",
    remote: "vits-piper-en_US-amy-low.tar.bz2",
    bytes: 67095344,
    checksum: "c70f5284a09a7fd4ed203b39b2ff51cac1432b422b852eb647b481dade3cf639",
};

// Speaker IDs belong to this exact v1.0 voices.bin. Japanese is omitted because
// this bundle has no Japanese text frontend.
#[rustfmt::skip]
const KOKORO_VOICES: &[(&str, u32)] = &[
    ("af_alloy", 0), ("af_aoede", 1), ("af_bella", 2), ("af_heart", 3),
    ("af_jessica", 4), ("af_kore", 5), ("af_nicole", 6), ("af_nova", 7),
    ("af_river", 8), ("af_sarah", 9), ("af_sky", 10), ("am_adam", 11),
    ("am_echo", 12), ("am_eric", 13), ("am_fenrir", 14), ("am_liam", 15),
    ("am_michael", 16), ("am_onyx", 17), ("am_puck", 18), ("am_santa", 19),
    ("bf_alice", 20), ("bf_emma", 21), ("bf_isabella", 22), ("bf_lily", 23),
    ("bm_daniel", 24), ("bm_fable", 25), ("bm_george", 26), ("bm_lewis", 27),
    ("ef_dora", 28), ("em_alex", 29), ("ff_siwis", 30),
    ("hf_alpha", 31), ("hf_beta", 32), ("hm_omega", 33), ("hm_psi", 34),
    ("if_sara", 35), ("im_nicola", 36),
    ("pf_dora", 42), ("pm_alex", 43), ("pm_santa", 44),
    ("zf_xiaobei", 45), ("zf_xiaoni", 46), ("zf_xiaoxiao", 47), ("zf_xiaoyi", 48),
    ("zm_yunjian", 49), ("zm_yunxi", 50), ("zm_yunxia", 51), ("zm_yunyang", 52),
];

pub fn voices() -> impl Iterator<Item = (&'static str, Engine)> {
    KOKORO_VOICES.iter().map(|v| (v.0, Engine::Kokoro)).chain([
        ("en_US-lessac-medium", Engine::Piper),
        ("en_US-amy-low", Engine::Piper),
    ])
}

pub fn voice_language(name: &str) -> Option<&'static str> {
    kokoro_metadata(name)
        .map(|v| v.0)
        .or_else(|| matches!(name, "en_US-lessac-medium" | "en_US-amy-low").then_some("en-us"))
}

pub fn voice_gender(name: &str) -> &'static str {
    kokoro_metadata(name).map_or("und", |v| v.1)
}

// Kokoro encodes locale and gender in its two-letter prefix. Only interpret
// registered presets: a saved voice named "af_example" has no such guarantee.
fn kokoro_metadata(name: &str) -> Option<(&'static str, &'static str)> {
    if !KOKORO_VOICES.iter().any(|v| v.0 == name) {
        return None;
    }
    let (prefix, _) = name.split_once('_')?;
    let [locale, gender] = prefix.as_bytes() else {
        return None;
    };
    let locale = match locale {
        b'a' => "en-us",
        b'b' => "en-gb",
        b'e' => "es",
        b'f' => "fr",
        b'h' => "hi",
        b'i' => "it",
        b'p' => "pt-br",
        b'z' => "zh",
        _ => return None,
    };
    let gender = match gender {
        b'f' => "female",
        b'm' => "male",
        _ => return None,
    };
    Some((locale, gender))
}
pub fn default_voice(engine: Engine) -> &'static str {
    if engine == Engine::Kokoro {
        "af_heart"
    } else {
        "en_US-lessac-medium"
    }
}

fn voice_name(config: &Config) -> Result<&str> {
    config
        .voice
        .as_ref()
        .map_or(Ok(default_voice(config.engine)), |p| {
            p.to_str().context("voice name must be UTF-8")
        })
}

pub fn validate(config: &Config) -> Result<()> {
    let name = voice_name(config)?;
    if !voices().any(|v| v == (name, config.engine)) {
        bail!(
            "unknown {} preset {name:?}; use --voices --engine {}",
            config.engine,
            config.engine
        );
    }
    if let Some(language) = &config.language {
        let language = language.to_lowercase().replace('_', "-");
        if config.engine == Engine::Kokoro {
            if !matches!(
                language.as_str(),
                "en" | "en-us"
                    | "en-gb"
                    | "zh"
                    | "cmn"
                    | "es"
                    | "fr"
                    | "hi"
                    | "it"
                    | "pt"
                    | "pt-br"
            ) {
                bail!("Kokoro supports en, en-us, en-gb, zh, es, fr, hi, it, pt-br");
            }
        } else if config.model.is_some() || !matches!(language.as_str(), "en" | "en-us") {
            bail!("Piper language is fixed by its model; omit --language and select a matching --model");
        }
    }
    Ok(())
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn onnx(dir: &Path) -> Result<PathBuf> {
    let mut files = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|p| p.extension().is_some_and(|e| e == "onnx"));
    if files.len() != 1 {
        bail!(
            "bundle {} must contain exactly one .onnx model",
            dir.display()
        );
    }
    Ok(files.remove(0))
}

fn check_bundle(dir: &Path, engine: Engine) -> Result<()> {
    onnx(dir)?;
    let mut files = vec![
        "tokens.txt",
        "espeak-ng-data/phontab",
        "espeak-ng-data/phonindex",
        "espeak-ng-data/phondata",
        "espeak-ng-data/intonations",
    ];
    if engine == Engine::Kokoro {
        files.extend([
            "voices.bin",
            "lexicon-us-en.txt",
            "lexicon-gb-en.txt",
            "lexicon-zh.txt",
        ]);
    }
    for file in files {
        if !dir.join(file).is_file() {
            bail!("bundle is missing {}", dir.join(file).display());
        }
    }
    Ok(())
}

pub fn model(config: &Config, quiet: bool) -> Result<PathBuf> {
    validate(config)?;
    if let Some(dir) = &config.model {
        let dir = ty_common::config::expand_user(dir);
        check_bundle(&dir, config.engine)?;
        return fs::canonicalize(dir).context("resolving local bundle");
    }
    let asset = match config.engine {
        Engine::Kokoro => &KOKORO,
        Engine::Piper if voice_name(config)? == "en_US-amy-low" => &AMY,
        Engine::Piper => &LESSAC,
        _ => unreachable!(),
    };
    let root = crate::model_root(config)?;
    let name = asset.local.trim_end_matches(".tar.bz2");
    let target = root.join(name);
    if target.exists() {
        check_bundle(&target, config.engine)?;
        return fs::canonicalize(target).context("resolving cached bundle");
    }
    crate::download(&root, BASE, asset, config.auto_download, quiet)?;
    let _progress = crate::Progress::new(format!("Unpacking {} model", config.engine), None, quiet);
    // Extract in an exclusive staging directory; never expose a partial bundle.
    let mut staging = None;
    for suffix in 0..1000 {
        let dir = root.join(format!(".{name}-{}-{suffix}", std::process::id()));
        match fs::create_dir(&dir) {
            Ok(()) => {
                staging = Some(Staging(dir));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    let staging = staging.context("creating bundle staging directory")?;
    native::run(
        native::command("tar")
            .args([
                "--extract",
                "--bzip2",
                "--no-same-owner",
                "--no-same-permissions",
                "--file",
            ])
            .arg(root.join(asset.local))
            .arg("--directory")
            .arg(&staging.0),
    )?;
    let extracted = staging.0.join(name);
    check_bundle(&extracted, config.engine)?;
    if let Err(e) = fs::rename(&extracted, &target) {
        // A concurrent invocation may have published the same verified bundle.
        if !target.is_dir() {
            return Err(e.into());
        }
        check_bundle(&target, config.engine)?;
    }
    // Keep the verified archive for repair/re-extraction without another download.
    fs::canonicalize(target).context("resolving installed bundle")
}

pub struct Synth {
    dir: PathBuf,
    engine: Engine,
    sid: u32,
    language: String,
}

impl Synth {
    pub fn new(config: &Config, dir: &Path) -> Result<Self> {
        validate(config)?;
        let (sid, language) = if config.engine == Engine::Kokoro {
            let name = voice_name(config)?;
            let v = KOKORO_VOICES
                .iter()
                .find(|v| v.0 == name)
                .context("unknown Kokoro voice")?;
            (
                v.1,
                voice_language(name).context("unknown Kokoro language prefix")?,
            )
        } else {
            (0, "en-us")
        };
        let language = config
            .language
            .as_deref()
            .unwrap_or(language)
            .to_lowercase()
            .replace('_', "-");
        let language = match language.as_str() {
            "en" => "en-us".into(),
            "cmn" => "zh".into(),
            "pt" => "pt-br".into(),
            _ => language,
        };
        Ok(Self {
            dir: dir.to_owned(),
            engine: config.engine,
            sid,
            language,
        })
    }

    pub fn stream(&self, text: &str) -> Result<(u32, crate::engine::AudioStream)> {
        // The native CLI accepts one positional argument. Fail clearly rather than
        // silently truncate text at Linux's per-argument limit.
        if text.len() > 100_000 {
            bail!("Kokoro/Piper requests are limited to 100,000 UTF-8 bytes; split this text into smaller requests");
        }
        let (wav, file) = Temporary::create(&std::env::temp_dir(), "sherpa.wav")?;
        drop(file);
        let mut cmd = native::command("sherpa-onnx-offline-tts");
        cmd.args([
            "--print-args=false",
            "--debug=false",
            "--provider=cpu",
            "--num-threads=2",
        ])
        .arg(format!("--sid={}", self.sid))
        .arg(format!("--output-filename={}", wav.0.display()));
        let prefix = if self.engine == Engine::Kokoro {
            "kokoro"
        } else {
            "vits"
        };
        for (flag, path) in [
            ("model", onnx(&self.dir)?),
            ("tokens", self.dir.join("tokens.txt")),
            ("data-dir", self.dir.join("espeak-ng-data")),
        ] {
            cmd.arg(format!("--{prefix}-{flag}={}", path.display()));
        }
        if self.engine == Engine::Kokoro {
            cmd.arg(format!(
                "--kokoro-voices={}",
                self.dir.join("voices.bin").display()
            ));
            if matches!(self.language.as_str(), "en-us" | "en-gb" | "zh") {
                let lexicon = match self.language.as_str() {
                    "en-gb" => "lexicon-gb-en.txt",
                    "zh" => "lexicon-zh.txt",
                    _ => "lexicon-us-en.txt",
                };
                cmd.arg(format!(
                    "--kokoro-lexicon={}",
                    self.dir.join(lexicon).display()
                ));
            } else {
                cmd.arg(format!("--kokoro-lang={}", self.language));
            }
        }
        cmd.arg("--").arg(text);
        native::run(&mut cmd)?;
        native::wav_stream(wav)
    }
}
