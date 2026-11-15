use std::io::BufWriter;
use std::path::Path;

use anyhow::{bail, Context, Result};
use libpulse_binding::{sample, stream};
use libpulse_simple_binding::Simple;

pub enum Output {
    Playback(Simple),
    Wav(hound::WavWriter<BufWriter<std::fs::File>>),
}

impl Output {
    pub fn new(path: Option<&Path>, sample_rate: u32) -> Result<Self> {
        if let Some(path) = path {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            return Ok(Self::Wav(
                hound::WavWriter::create(path, spec).context("creating output WAV")?,
            ));
        }
        let spec = sample::Spec {
            format: sample::Format::FLOAT32NE,
            channels: 1,
            rate: sample_rate,
        };
        // Keep only ~160 ms queued. Automatic prebuffering waits for initial
        // inference and restarts after underruns. With prebuf=0 the playback clock
        // runs ahead of slow inference and late samples can be silently discarded.
        let attr = libpulse_binding::def::BufferAttr {
            maxlength: u32::MAX,
            tlength: sample_rate * 4 * 160 / 1000,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: u32::MAX,
        };
        Ok(Self::Playback(
            Simple::new(
                None,
                "tytts",
                stream::Direction::Playback,
                None,
                "Speech",
                &spec,
                None,
                Some(&attr),
            )
            .context("opening audio output; start PulseAudio or PipeWire's PulseAudio server")?,
        ))
    }

    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        match self {
            Self::Playback(player) => {
                let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_ne_bytes()).collect();
                player
                    .write(&bytes)
                    .context("streaming audio to playback")?;
            }
            Self::Wav(writer) => {
                for &sample in samples {
                    writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
                }
                // Publish each generated block before the next inference step.
                writer.flush()?;
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Result<()> {
        match self {
            Self::Playback(player) => player.drain().context("finishing audio playback"),
            Self::Wav(writer) => writer.finalize().context("finalizing WAV"),
        }
    }
}

/// Play a completed WAV in bounded blocks, independent of inference speed.
pub fn play_wav(path: &Path) -> Result<()> {
    let mut reader = hound::WavReader::open(path).context("opening generated speech")?;
    let mut output = Output::new(None, reader.spec().sample_rate)?;
    let mut samples = reader.samples::<i16>();
    loop {
        let block = samples
            .by_ref()
            .take(8192)
            .map(|s| s.map(|s| s as f32 / 32768.0))
            .collect::<Result<Vec<_>, _>>()?;
        if block.is_empty() {
            break;
        }
        output.write(&block)?;
    }
    output.finish()
}

/// One Sonic stream for the entire utterance preserves pitch and continuity
/// when --speed differs from 1. At normal speed the audio is passed through.
pub struct Tempo(sonic_sys::sonicStream);

impl Tempo {
    pub fn new(sample_rate: u32, speed: f64) -> Result<Self> {
        // SAFETY: Valid mono sample rate; the returned allocation is owned here.
        let stream = unsafe { sonic_sys::sonicCreateStream(sample_rate as i32, 1) };
        if stream.is_null() {
            bail!("allocating speech speed processor");
        }
        unsafe { sonic_sys::sonicSetSpeed(stream, speed as f32) };
        Ok(Self(stream))
    }

    pub fn write(&mut self, samples: &mut [f32]) -> Result<()> {
        let count = i32::try_from(samples.len()).context("audio block too large")?;
        // SAFETY: self owns the live stream; samples has count mono frames.
        if unsafe { sonic_sys::sonicWriteFloatToStream(self.0, samples.as_mut_ptr(), count) } == 0 {
            bail!("processing speech speed");
        }
        Ok(())
    }

    pub fn read(&mut self) -> Vec<f32> {
        // SAFETY: All calls access this owned stream exclusively; buffer capacity
        // matches the number of available mono samples reported by Sonic.
        unsafe {
            let count = sonic_sys::sonicSamplesAvailable(self.0);
            let mut samples = vec![0.0; count as usize];
            let read = sonic_sys::sonicReadFloatFromStream(self.0, samples.as_mut_ptr(), count);
            samples.truncate(read as usize);
            samples
        }
    }

    pub fn flush(&mut self) -> Result<()> {
        if unsafe { sonic_sys::sonicFlushStream(self.0) } == 0 {
            bail!("flushing speech speed processor");
        }
        Ok(())
    }
}

impl Drop for Tempo {
    fn drop(&mut self) {
        // SAFETY: This is the sole owner and no references outlive it.
        unsafe { sonic_sys::sonicDestroyStream(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_blocks_are_readable_before_output_is_finished() -> Result<()> {
        let (file, handle) = crate::Temporary::create(&std::env::temp_dir(), "stream-test.wav")?;
        drop(handle);
        let mut output = Output::new(Some(&file.0), 24000)?;
        output.write(&[0.0, 0.5, -0.5])?;
        let first = hound::WavReader::open(&file.0)?;
        assert_eq!(first.duration(), 3);
        drop(first);
        output.write(&[1.0, -1.0])?;
        output.finish()?;
        let mut reader = hound::WavReader::open(&file.0)?;
        assert_eq!(reader.spec().sample_rate, 24000);
        assert_eq!(
            reader.samples::<i16>().collect::<Result<Vec<_>, _>>()?,
            [0, 16383, -16383, 32767, -32767]
        );
        Ok(())
    }

    #[test]
    fn streamed_tempo_changes_duration_without_changing_pitch() -> Result<()> {
        const RATE: u32 = 24000;
        const HZ: f32 = 220.0;
        for speed in [0.5, 1.5, 4.0] {
            let mut processor = Tempo::new(RATE, speed)?;
            let mut input: Vec<f32> = (0..RATE * 2)
                .map(|i| (i as f32 * std::f32::consts::TAU * HZ / RATE as f32).sin() * 0.5)
                .collect();
            let mut output = Vec::new();
            for frame in input.chunks_mut(1920) {
                processor.write(frame)?;
                output.extend(processor.read());
            }
            // Sonic must produce audio before EOF, including at non-default speeds.
            assert!(!output.is_empty());
            processor.flush()?;
            output.extend(processor.read());
            let seconds = output.len() as f64 / RATE as f64;
            assert!((seconds - 2.0 / speed).abs() < 0.05, "{speed}: {seconds}s");
            let crossings = output
                .windows(2)
                .filter(|p| p[0] < 0.0 && p[1] >= 0.0)
                .count();
            let hz = crossings as f64 / seconds;
            assert!((hz - HZ as f64).abs() < 10.0, "{speed}: {hz}Hz");
        }
        Ok(())
    }
}
