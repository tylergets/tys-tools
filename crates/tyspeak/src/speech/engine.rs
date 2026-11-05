//! The loaded speech model. All calls block; run them off the GTK thread.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use transcribe_cpp::{Model, RunOptions, Session, SessionOptions};

use crate::speech::audio::SAMPLE_RATE;

struct Loaded {
    id: String,
    session: Session,
    languages: Vec<String>,
    /// Longest audio one run accepts, in samples (`None` = no practical limit).
    max_samples: Option<usize>,
}

/// Holds at most one model. Cheap to clone; clones share the model.
#[derive(Clone, Default)]
pub struct Engine {
    loaded: Arc<Mutex<Option<Loaded>>>,
    /// Bumped on every load request so a superseded load is skipped.
    generation: Arc<AtomicU64>,
}

impl Engine {
    /// Claim the next load; pass the result to [`Self::load`].
    pub fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// Load `path` as model `id`, replacing the current model. Returns `false` when a newer
    /// load was requested meanwhile (nothing was loaded).
    pub fn load(&self, generation: u64, id: &str, path: &Path, threads: u32) -> Result<bool> {
        let mut loaded = self.loaded.lock().unwrap();
        if !self.is_current(generation) {
            return Ok(false);
        }
        if loaded.as_ref().is_some_and(|current| current.id == id) {
            return Ok(true);
        }
        // Free the old model first so two never sit in memory together.
        *loaded = None;
        let model = Model::load(path)
            .map_err(|err| anyhow::anyhow!("{err}"))
            .with_context(|| format!("failed to load {}", path.display()))?;
        let capabilities = model
            .capabilities()
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        let session = model
            .session_with(&SessionOptions {
                n_threads: threads as i32,
                ..SessionOptions::default()
            })
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        let max_ms = session
            .limits()
            .map(|limits| limits.effective_max_audio_ms)
            .unwrap_or(capabilities.max_audio_ms);
        eprintln!(
            "loaded {id}: {} {} on {}",
            model.arch(),
            model.variant(),
            model.backend()
        );
        *loaded = Some(Loaded {
            id: id.to_string(),
            session,
            languages: capabilities.languages,
            max_samples: (max_ms > 0).then(|| (max_ms as usize) * (SAMPLE_RATE as usize) / 1000),
        });
        Ok(true)
    }

    /// Transcribe 16 kHz mono audio. Audio longer than the model takes in one run is split at
    /// the quietest moments. `language` is passed only to models that list it.
    pub fn transcribe(&self, pcm: &[f32], language: Option<&str>) -> Result<String> {
        let mut guard = self.loaded.lock().unwrap();
        let loaded = guard.as_mut().context("no speech model is loaded")?;
        let options = RunOptions {
            language: language
                .filter(|language| loaded.languages.iter().any(|known| known == language))
                .map(str::to_string),
            ..RunOptions::default()
        };
        let mut parts = Vec::new();
        for range in split_points(pcm, loaded.max_samples) {
            let transcript = loaded
                .session
                .run(&pcm[range], &options)
                .map_err(|err| anyhow::anyhow!("{err}"))
                .context("transcription failed")?;
            let text = transcript.text.trim();
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }
        Ok(parts.join(" "))
    }
}

/// Ranges covering `pcm`, each at most `max` samples, cut at the quietest 50 ms in the last
/// quarter of each piece.
#[allow(clippy::single_range_in_vec_init)]
fn split_points(pcm: &[f32], max: Option<usize>) -> Vec<std::ops::Range<usize>> {
    const WINDOW: usize = (SAMPLE_RATE / 20) as usize;
    let Some(max) = max.filter(|&max| max > WINDOW * 4) else {
        return vec![0..pcm.len()];
    };
    let mut ranges = Vec::new();
    let mut start = 0;
    while pcm.len() - start > max {
        let search_from = start + max * 3 / 4;
        let search_to = start + max - WINDOW;
        let cut = (search_from..=search_to)
            .step_by(WINDOW / 2)
            .min_by(|&a, &b| energy(&pcm[a..a + WINDOW]).total_cmp(&energy(&pcm[b..b + WINDOW])))
            .map(|at| at + WINDOW / 2)
            .unwrap_or(start + max);
        ranges.push(start..cut);
        start = cut;
    }
    ranges.push(start..pcm.len());
    ranges
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum()
}

#[cfg(test)]
mod tests {
    use super::split_points;

    #[test]
    fn short_audio_is_one_piece() {
        assert_eq!(split_points(&[0.0; 100], None), vec![0..100]);
        assert_eq!(split_points(&[0.0; 100], Some(16_000)), vec![0..100]);
    }

    #[test]
    fn long_audio_is_cut_in_the_quiet() {
        let max = 16_000;
        // Loud everywhere except a quiet gap around sample 13_000.
        let mut pcm = vec![0.5f32; 40_000];
        pcm[12_800..13_600].iter_mut().for_each(|s| *s = 0.0);
        let ranges = split_points(&pcm, Some(max));
        assert!(ranges.iter().all(|range| range.len() <= max));
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, pcm.len());
        assert!((12_800..13_600).contains(&ranges[0].end), "{ranges:?}");
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
    }
}
