use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::Engine;

pub fn builtins() -> impl Iterator<Item = (&'static str, Engine)> {
    [("alba", Engine::Pocket), ("savannah", Engine::Zonos)]
        .into_iter()
        .chain(crate::sherpa::voices())
        .chain(crate::qwen::voices())
}

fn language(name: &str) -> &'static str {
    match name {
        "alba" => "en",
        "savannah" => "en-us",
        _ => crate::sherpa::voice_language(name)
            .or_else(|| crate::qwen::metadata(name).map(|v| v.0))
            .unwrap_or("und"),
    }
}

fn gender(name: &str) -> &'static str {
    crate::qwen::metadata(name).map_or_else(|| crate::sherpa::voice_gender(name), |v| v.1)
}

// Preserve regions while accepting BCP-47 tags and POSIX locales.
fn normalize_language(locale: &str) -> String {
    let value = locale.trim().to_lowercase().replace('_', "-");
    let value = value.split(['.', '@']).next().unwrap_or("");
    let (language, suffix) = value.split_once('-').unwrap_or((value, ""));
    let language = match language {
        "c" | "posix" => return "en-us".into(),
        "cmn" => "zh",
        "nb" | "nn" => "no",
        language => language,
    };
    if suffix.is_empty() {
        language.into()
    } else {
        format!("{language}-{suffix}")
    }
}

fn language_matches(selected: &str, voice: &str) -> bool {
    let voice = normalize_language(voice);
    selected == voice
        || voice
            .strip_prefix(selected)
            .is_some_and(|rest| rest.starts_with('-'))
        || selected
            .strip_prefix(&voice)
            .is_some_and(|rest| rest.starts_with('-'))
}

fn os_language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "en-us".into())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Voice {
    pub version: u32,
    pub engine: Engine,
    pub model: String,
}

pub fn root() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .context("cannot determine data directory")?
        .join("tytts/voices"))
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        || builtins().any(|v| v.0 == name)
    {
        bail!("voice names must be 1–64 ASCII letters, digits, '-' or '_'; built-in voice names are reserved");
    }
    Ok(())
}

pub fn load(name: &str) -> Result<(Voice, PathBuf)> {
    validate_name(name)?;
    let dir = root()?.join(name);
    let voice: Voice = serde_json::from_slice(
        &fs::read(dir.join("manifest.json"))
            .with_context(|| format!("unknown or incomplete voice {name:?}; use --voices"))?,
    )?;
    if voice.version != 1 {
        bail!("unsupported voice format version {}", voice.version);
    }
    let file = dir.join(match voice.engine {
        Engine::Pocket => "voice.safetensors",
        Engine::Zonos => "embedding.npy",
        Engine::Qwen => "clone.spk",
        Engine::Kokoro | Engine::Piper => {
            bail!("saved clones are not supported for {}", voice.engine)
        }
    });
    if !file.is_file() {
        bail!("voice {name:?} is missing {}", file.display());
    }
    Ok((voice, file))
}

pub fn list(filter: Option<Engine>, requested: Option<&str>, all_languages: bool) -> Result<()> {
    let selected = if all_languages {
        None
    } else {
        let group = normalize_language(&requested.map(str::to_owned).unwrap_or_else(os_language));
        if group.is_empty() {
            bail!("language filter must not be empty");
        }
        Some(group)
    };
    let mut rows = vec![(
        "NAME".to_owned(),
        "ENGINE".to_owned(),
        "LANGUAGE",
        "GENDER",
        "KIND",
    )];
    for (name, engine) in builtins() {
        let language = language(name);
        if filter.is_none_or(|f| f == engine)
            && selected
                .as_ref()
                .is_none_or(|s| language_matches(s, language))
        {
            rows.push((
                name.to_owned(),
                engine.to_string(),
                language,
                gender(name),
                "built-in",
            ));
        }
    }
    let root = root()?;
    let mut entries = if root.exists() {
        fs::read_dir(root)?.collect::<std::io::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        match load(&name) {
            Ok((voice, _)) if filter.is_none_or(|f| f == voice.engine) => {
                // Existing manifests carry no reference-language metadata. Keep
                // these reusable multilingual voices visible rather than guess.
                rows.push((name, voice.engine.to_string(), "und", "und", "saved"));
            }
            Ok(_) => {}
            Err(err) => eprintln!("Skipping {name:?}: {err:#}"),
        }
    }
    let name_width = rows.iter().map(|r| r.0.len()).max().unwrap_or(0);
    let engine_width = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
    let language_width = rows.iter().map(|r| r.2.len()).max().unwrap_or(0);
    let gender_width = rows.iter().map(|r| r.3.len()).max().unwrap_or(0);
    for (name, engine, language, gender, kind) in rows {
        println!(
            "{name:<name_width$}  {engine:<engine_width$}  {language:<language_width$}  {gender:<gender_width$}  {kind}"
        );
    }
    Ok(())
}

pub fn info(name: &str) -> Result<()> {
    if let Some((_, engine)) = builtins().find(|v| v.0 == name) {
        println!(
            "{name}: built-in {engine} voice\nLanguage: {}\nGender: {}",
            language(name),
            gender(name)
        );
        return Ok(());
    }
    let (voice, path) = load(name)?;
    println!(
        "Name: {name}\nEngine: {}\nModel: {}\nFile: {}",
        voice.engine,
        voice.model,
        path.display()
    );
    Ok(())
}

pub fn remove(name: &str) -> Result<()> {
    validate_name(name)?;
    // Never follow a voice-directory symlink during removal.
    let path = root()?.join(name);
    if fs::symlink_metadata(&path)?.file_type().is_symlink() {
        bail!("refusing to remove a symlink voice directory");
    }
    fs::remove_dir_all(&path).with_context(|| format!("removing voice {name:?}"))?;
    println!("Removed {name}");
    Ok(())
}

/// A hidden directory is published only after encoding and metadata succeed.
pub fn create(
    name: &str,
    voice: &Voice,
    source: &Path,
    encode: impl FnOnce(&Path, &Path) -> Result<()>,
) -> Result<()> {
    validate_name(name)?;
    let root = root()?;
    fs::create_dir_all(&root)?;
    let target = root.join(name);
    // An exclusive lock prevents concurrent creators from replacing one another.
    let lock_path = root.join(format!(".{name}.lock"));
    let _lock_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .context("voice creation already in progress (or stale .lock file in voices directory)")?;
    let _lock = crate::Temporary(lock_path);
    if target.exists() {
        bail!("voice {name:?} already exists; use a different name or --remove-voice");
    }
    let staging = root.join(format!(".{name}-{}", std::process::id()));
    fs::create_dir(&staging)?;
    struct Pending(PathBuf);
    impl Drop for Pending {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _pending = Pending(staging.clone());
    // Preserve the original container extension for future re-encoding.
    let extension = source
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    let reference = staging.join(format!("reference.{extension}"));
    fs::copy(source, &reference)?;
    encode(&staging, &reference)?;
    fs::write(
        staging.join("manifest.json"),
        serde_json::to_vec_pretty(voice)?,
    )?;
    fs::rename(&staging, &target)?;
    println!("Saved {name} ({})", voice.engine);
    Ok(())
}
