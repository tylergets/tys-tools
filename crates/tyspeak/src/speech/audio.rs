//! Microphone capture through `parec` (PulseAudio / PipeWire-pulse) as 16 kHz mono s16le, the
//! format the speech models take.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

pub const SAMPLE_RATE: u32 = 16_000;
/// Samples per level update (50 ms).
const CHUNK_SAMPLES: usize = (SAMPLE_RATE / 20) as usize;
/// Levels at or below this many dBFS read as silence on the meter.
const METER_FLOOR_DB: f32 = -60.0;

/// The most recent audio level, 0.0 (silent) to 1.0 (full scale), shared across threads.
#[derive(Debug, Clone, Default)]
pub struct Level(Arc<AtomicU32>);

impl Level {
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    fn set(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

/// A running microphone recording.
pub struct Recorder {
    child: Child,
    reader: Option<JoinHandle<Vec<i16>>>,
    level: Level,
    started: Instant,
}

impl Recorder {
    /// Start recording `source` (a PulseAudio source name, or `default`).
    pub fn start(source: &str) -> Result<Self> {
        let mut command = Command::new("parec");
        command.args([
            "--raw",
            "--format=s16le",
            "--channels=1",
            "--latency-msec=20",
            "--client-name=tyspeak",
        ]);
        command.arg(format!("--rate={SAMPLE_RATE}"));
        if source != "default" {
            command.arg(format!("--device={source}"));
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to launch parec (is PulseAudio or pipewire-pulse running?)")?;
        let mut stdout = child.stdout.take().context("parec stdout unavailable")?;

        let level = Level::default();
        let publish = level.clone();
        let reader = thread::spawn(move || {
            let mut samples = Vec::with_capacity(SAMPLE_RATE as usize * 30);
            let mut buffer = vec![0u8; CHUNK_SAMPLES * 2];
            let mut pending = Vec::new();
            loop {
                let read = match stdout.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                pending.extend_from_slice(&buffer[..read]);
                let whole = pending.len() / 2 * 2;
                let chunk: Vec<i16> = pending[..whole]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| i16::from_le_bytes(*pair))
                    .collect();
                pending.drain(..whole);
                publish.set(peak_level(&chunk));
                samples.extend(chunk);
            }
            publish.set(0.0);
            samples
        });

        Ok(Self {
            child,
            reader: Some(reader),
            level,
            started: Instant::now(),
        })
    }

    pub fn level(&self) -> f32 {
        self.level.get()
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// `Some(message)` if parec exited on its own (bad source, no server, ...).
    pub fn failure(&mut self) -> Option<String> {
        let status = self.child.try_wait().ok().flatten()?;
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        let stderr = stderr.trim();
        Some(if stderr.is_empty() {
            format!("parec exited ({status})")
        } else {
            format!("parec exited ({status}): {stderr}")
        })
    }

    /// Stop recording and return the audio as f32 samples in -1..=1. Blocks briefly.
    pub fn finish(mut self) -> Vec<f32> {
        self.kill();
        let samples = self
            .reader
            .take()
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        to_f32(&samples)
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Rate of the start chime handed to `pacat`.
const CHIME_RATE: u32 = 48_000;

/// Which chime to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chime {
    /// Rising two notes: recording started.
    Start,
    /// Falling two notes: recording stopped.
    Stop,
}

/// Play `chime` through `pacat` without blocking. Failures are ignored: a missing sound
/// shouldn't get in the way of dictation.
pub fn play_chime(chime: Chime) {
    thread::spawn(move || {
        let Ok(mut child) = Command::new("pacat")
            .args([
                "--playback",
                "--raw",
                "--format=s16le",
                "--channels=1",
                "--client-name=tyspeak",
                "--stream-name=chime",
            ])
            .arg(format!("--rate={CHIME_RATE}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let bytes: Vec<u8> = chime_samples(chime)
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect();
            let _ = stdin.write_all(&bytes);
        }
        let _ = child.wait();
    });
}

/// Two soft sine notes (A5 and E6, rising or falling) with a quick attack and exponential
/// decay.
fn chime_samples(chime: Chime) -> Vec<i16> {
    const LOW: f32 = 880.0;
    const HIGH: f32 = 1318.5;
    const AMPLITUDE: f32 = 0.22;
    const ATTACK_SECS: f32 = 0.004;
    let notes = match chime {
        Chime::Start => [(LOW, 0.07), (HIGH, 0.11)],
        Chime::Stop => [(HIGH, 0.07), (LOW, 0.11)],
    };
    let rate = CHIME_RATE as f32;
    notes
        .into_iter()
        .flat_map(|(freq, secs)| {
            let len = (secs * rate) as usize;
            (0..len).map(move |i| {
                let t = i as f32 / rate;
                let attack = (t / ATTACK_SECS).min(1.0);
                let decay = (-t * 5.0 / secs).exp();
                let wave = (std::f32::consts::TAU * freq * t).sin();
                (wave * attack * decay * AMPLITUDE * f32::from(i16::MAX)) as i16
            })
        })
        .collect()
}

pub fn to_f32(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .map(|&sample| f32::from(sample) / 32768.0)
        .collect()
}

/// Peak of `samples` in dBFS (`-inf` for silence).
pub fn peak_db(samples: &[f32]) -> f32 {
    let peak = samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
    20.0 * peak.log10()
}

/// Peak of a chunk on a log scale from 0.0 (<= -60 dBFS) to 1.0 (0 dBFS).
fn peak_level(samples: &[i16]) -> f32 {
    let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    if peak == 0 {
        return 0.0;
    }
    let db = 20.0 * (f32::from(peak) / f32::from(i16::MAX)).log10();
    ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

/// A single block character showing `level` (0.0..=1.0).
pub fn level_meter(level: f32) -> char {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let index = (level.clamp(0.0, 1.0) * (BLOCKS.len() - 1) as f32).round() as usize;
    BLOCKS[index]
}

/// Mix to mono and linearly resample to [`SAMPLE_RATE`] (for `tyspeak transcribe` on WAVs).
pub fn to_model_input(interleaved: &[f32], channels: usize, rate: u32) -> Vec<f32> {
    let channels = channels.max(1);
    let mono: Vec<f32> = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    if rate == SAMPLE_RATE || mono.is_empty() {
        return mono;
    }
    let ratio = f64::from(rate) / f64::from(SAMPLE_RATE);
    let len = (mono.len() as f64 / ratio).floor() as usize;
    (0..len)
        .map(|index| {
            let position = index as f64 * ratio;
            let base = position.floor() as usize;
            let next = (base + 1).min(mono.len() - 1);
            let fraction = (position - base as f64) as f32;
            mono[base] * (1.0 - fraction) + mono[next] * fraction
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        chime_samples, level_meter, peak_db, peak_level, to_model_input, Chime, CHIME_RATE,
    };

    #[test]
    fn meter_levels() {
        assert_eq!(peak_level(&[0, 0]), 0.0);
        assert_eq!(peak_level(&[0, i16::MIN, 5]), 1.0);
        assert!((peak_level(&[i16::MAX / 2]) - 0.9).abs() < 0.01);
        assert_eq!(level_meter(0.0), '▁');
        assert_eq!(level_meter(7.0), '█');
    }

    #[test]
    fn peak_db_of_half_scale() {
        assert!((peak_db(&[0.0, -0.5, 0.25]) + 6.02).abs() < 0.01);
        assert_eq!(peak_db(&[0.0]), f32::NEG_INFINITY);
    }

    #[test]
    fn chimes_are_short_and_quiet() {
        for chime in [Chime::Start, Chime::Stop] {
            let samples = chime_samples(chime);
            assert!(samples.len() < CHIME_RATE as usize / 4);
            let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
            assert!(peak > 0 && peak < i16::MAX as u16 / 4);
        }
        assert_ne!(chime_samples(Chime::Start), chime_samples(Chime::Stop));
    }

    #[test]
    fn resamples_and_downmixes() {
        let stereo = [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        assert_eq!(to_model_input(&stereo, 2, 16_000), vec![0.5; 4]);
        let at_32k: Vec<f32> = (0..32).map(|i| i as f32).collect();
        let resampled = to_model_input(&at_32k, 1, 32_000);
        assert_eq!(resampled.len(), 16);
        assert_eq!(resampled[3], 6.0);
    }
}
