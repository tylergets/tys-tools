//! The speech-to-text models tyspeak offers. These are the GGUF builds Hex
//! (github.com/anomalyco/hex, `src/transcription/artifacts.rs`) ships, pinned to the same
//! Hugging Face revisions and checksums, and run with transcribe.cpp like Hex does.

use std::path::{Path, PathBuf};

pub const DEFAULT_MODEL: &str = "parakeet-unified-en";

#[derive(Debug, PartialEq, Eq)]
pub struct ModelInfo {
    /// Stable id used in config, state and over D-Bus.
    pub id: &'static str,
    pub name: &'static str,
    /// Short note on languages and speed, shown in `tyspeak models`.
    pub summary: &'static str,
    pub repository: &'static str,
    pub revision: &'static str,
    pub filename: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "parakeet-unified-en",
        name: "Parakeet Unified English",
        summary: "English; fastest and most accurate (Hex's default)",
        repository: "handy-computer/parakeet-unified-en-0.6b-gguf",
        revision: "7e948f21b7bdbac698d3318db9d350f1096f3b6c",
        filename: "parakeet-unified-en-0.6b-Q8_0.gguf",
        bytes: 731_357_568,
        sha256: "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795",
    },
    ModelInfo {
        id: "parakeet-v2",
        name: "Parakeet v2",
        summary: "English",
        repository: "handy-computer/parakeet-tdt-0.6b-v2-gguf",
        revision: "07cee0616125a08ef619729bb47f40ef747e4bc4",
        filename: "parakeet-tdt-0.6b-v2-Q8_0.gguf",
        bytes: 729_574_912,
        sha256: "f0d0e99cebb6d3b83f1f7069b82b5d3c2e39a54545b0da039cb4bafd9c4e5caa",
    },
    ModelInfo {
        id: "parakeet-v3",
        name: "Parakeet v3",
        summary: "25 European languages, detected automatically",
        repository: "handy-computer/parakeet-tdt-0.6b-v3-gguf",
        revision: "85ac09ea12fc4b1112fa76810059364bc6adc9de",
        filename: "parakeet-tdt-0.6b-v3-Q8_0.gguf",
        bytes: 739_508_576,
        sha256: "5859f77944efcd8eafa23a6350731960b2b55b2203df51f319665c807d802cc7",
    },
    ModelInfo {
        id: "whisper-large-v3-turbo",
        name: "Whisper large-v3-turbo",
        summary: "99 languages; slower",
        repository: "handy-computer/whisper-large-v3-turbo-gguf",
        revision: "d222c9f621c1128299248f2ded4d8a1820519780",
        filename: "whisper-large-v3-turbo-Q8_0.gguf",
        bytes: 886_381_824,
        sha256: "d5e65f2b0828802ae2c231673d31982cebe3a778c95d9494a9f3efee6bd17448",
    },
    ModelInfo {
        id: "qwen3-asr-0.6b",
        name: "Qwen3-ASR 0.6B",
        summary: "Multilingual, detected automatically",
        repository: "handy-computer/Qwen3-ASR-0.6B-gguf",
        revision: "e4e16599b900eb0cb36e524514756bb92eb092b7",
        filename: "Qwen3-ASR-0.6B-Q8_0.gguf",
        bytes: 850_423_456,
        sha256: "f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8",
    },
    ModelInfo {
        id: "sensevoice-small",
        name: "SenseVoice Small",
        summary: "Chinese, Cantonese, English, Japanese, Korean; smallest download",
        repository: "handy-computer/SenseVoiceSmall-gguf",
        revision: "4a08b8e900b38a977e32eb08d5d0697d6e72ba04",
        filename: "SenseVoiceSmall-Q8_0.gguf",
        bytes: 252_684_608,
        sha256: "6c759ee4c9748c9b3f7a5a60ca74f0f7e685fb9d45d1378fce7cfd62f59adf29",
    },
];

pub fn find(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|model| model.id == id)
}

impl ModelInfo {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repository, self.revision, self.filename
        )
    }

    pub fn path_in(&self, dir: &Path) -> PathBuf {
        dir.join(self.filename)
    }

    /// Present with the expected size. The checksum is verified once, when downloading.
    pub fn is_downloaded(&self, dir: &Path) -> bool {
        std::fs::metadata(self.path_in(dir))
            .map(|meta| meta.is_file() && meta.len() == self.bytes)
            .unwrap_or(false)
    }

    pub fn size_label(&self) -> String {
        human_size(self.bytes)
    }
}

/// `1.4 GB`, `731 MB`, `12 KB`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::{find, human_size, DEFAULT_MODEL, MODELS};

    #[test]
    fn catalog_is_consistent() {
        assert!(find(DEFAULT_MODEL).is_some());
        for (index, model) in MODELS.iter().enumerate() {
            assert_eq!(model.sha256.len(), 64, "{}", model.id);
            assert!(model.filename.ends_with(".gguf"), "{}", model.id);
            assert!(
                MODELS[..index].iter().all(|other| other.id != model.id),
                "duplicate id {}",
                model.id
            );
        }
    }

    #[test]
    fn builds_pinned_urls() {
        assert_eq!(
            find("parakeet-v2").unwrap().url(),
            "https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v2-gguf/resolve/\
             07cee0616125a08ef619729bb47f40ef747e4bc4/parakeet-tdt-0.6b-v2-Q8_0.gguf"
        );
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(12_345), "12.3 KB");
        assert_eq!(human_size(731_357_568), "731 MB");
        assert_eq!(human_size(1_400_000_000), "1.4 GB");
    }
}
