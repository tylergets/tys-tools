use std::path::Path;

use anyhow::{Context, Result};
use ptts::preprocess::Lang;
use ptts::synth::{DeviceKind, Synth};
use sentencepiece_rs::SentencePieceProcessor;

struct Tokenizer(SentencePieceProcessor);

impl ptts::Tokenizer for Tokenizer {
    fn encode(&self, text: &str) -> xn::Result<Vec<u32>> {
        self.0
            .encode_to_ids(text)
            .map(|ids| ids.into_iter().map(|id| id as u32).collect())
            .map_err(xn::Error::msg)
    }

    fn decode(&self, tokens: &[u32]) -> xn::Result<String> {
        self.0
            .decode_ids(&tokens.iter().map(|&id| id as usize).collect::<Vec<_>>())
            .map_err(xn::Error::msg)
    }
}

// Architecture of Kyutai's b6369a24 checkpoint, matching the upstream Pocket TTS
// January 2026 config. Kept in Rust so it is included in isolated crate builds.
fn default_config() -> Result<ptts::tts_model::TTSConfig> {
    Ok(serde_json::from_value(serde_json::json!({
        "flow_lm": {
            "d_model": 1024, "num_heads": 16, "num_layers": 6,
            "dim_feedforward": 4096, "max_period": 10000.0,
            "n_bins": 4000, "lut_dim": 1024, "flow_dim": 512,
            "flow_depth": 6, "ldim": 32
        },
        "mimi": {
            "channels": 1, "sample_rate": 24000, "frame_rate": 12.5,
            "dimension": 512, "quantizer_dimension": 32,
            "quantizer_output_dimension": 512, "n_filters": 64,
            "n_residual_layers": 1, "ratios": [6, 5, 4],
            "kernel_size": 7, "last_kernel_size": 3, "residual_kernel_size": 3,
            "dilation_base": 2, "compress": 2, "transformer_d_model": 512,
            "transformer_num_heads": 8, "transformer_num_layers": 2,
            "transformer_layer_scale": 0.01, "transformer_context": 250,
            "transformer_max_period": 10000.0,
            "transformer_dim_feedforward": 2048, "downsample_channel_wise": false
        },
        "lsd_decode_steps": 1, "eos_threshold": -4.0, "model_id": null
    }))?)
}

fn load_pocket(dir: &Path, voice: &Path, custom: bool) -> Result<Synth> {
    let config = if custom {
        ptts::checkpoint::read_config(dir.join("config.json"))?
    } else {
        default_config()?
    };
    let tokenizer = Tokenizer(
        SentencePieceProcessor::open(dir.join("tokenizer.model"))
            .context("loading Pocket TTS tokenizer")?,
    );
    Synth::builder(config, dir.join("model.safetensors"), Lang::En)
        .device(DeviceKind::Cpu)
        .tokenizer(Box::new(tokenizer))
        .add_voice("default", voice)
        .build()
        .context("loading Pocket TTS weights and voice")
}

pub type AudioStream = Box<dyn Iterator<Item = Result<Vec<f32>>>>;

/// The output pipeline only needs a sample rate and a fallible PCM stream.
/// Keep the selected runtime alive until its stream has finished.
pub enum Loaded {
    Pocket(Synth),
    Zonos(crate::zonos::Server),
    Sherpa(crate::sherpa::Synth),
    Qwen(crate::qwen::Synth),
}

impl Loaded {
    pub fn load(config: &crate::Config, dir: &Path, voice: &Path) -> Result<Self> {
        match config.engine {
            crate::Engine::Qwen => Ok(Self::Qwen(crate::qwen::Synth::new(config, dir)?)),
            crate::Engine::Pocket => Ok(Self::Pocket(load_pocket(
                dir,
                voice,
                config.model.is_some(),
            )?)),
            crate::Engine::Zonos => Ok(Self::Zonos(crate::zonos::Server::start(
                dir,
                config.gpu,
                config.language.as_deref().unwrap_or("en_us"),
            )?)),
            crate::Engine::Kokoro | crate::Engine::Piper => {
                Ok(Self::Sherpa(crate::sherpa::Synth::new(config, dir)?))
            }
        }
    }

    pub fn stream(&self, text: &str, voice: &Path, language: &str) -> Result<(u32, AudioStream)> {
        match self {
            Self::Pocket(synth) => Ok((
                synth.sample_rate(),
                Box::new(synth.stream(text)?.map(|chunk| chunk.map_err(Into::into))),
            )),
            Self::Zonos(server) => Ok((44100, Box::new(server.stream(text, voice, language)?))),
            Self::Sherpa(synth) => synth.stream(text),
            Self::Qwen(synth) => synth.stream(text),
        }
    }

    pub fn check(&mut self) -> Result<()> {
        match self {
            Self::Pocket(_) | Self::Sherpa(_) | Self::Qwen(_) => Ok(()),
            Self::Zonos(server) => server.check(),
        }
    }
}
