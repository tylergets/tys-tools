//! A small WAV reader for `tyspeak transcribe`: integer PCM (8, 16, 24 or 32 bit) and float
//! (32 or 64 bit), including WAVE_FORMAT_EXTENSIBLE files.

use std::path::Path;

use anyhow::{bail, Context, Result};

const PCM: u16 = 1;
const FLOAT: u16 = 3;
const EXTENSIBLE: u16 = 0xfffe;

/// Decoded audio: interleaved samples in -1.0..1.0.
pub struct Wav {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

pub fn read(path: &Path) -> Result<Wav> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to open {}", path.display()))?;
    parse(&bytes).with_context(|| format!("{} isn't a WAV file we can read", path.display()))
}

pub fn parse(bytes: &[u8]) -> Result<Wav> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a RIFF/WAVE file");
    }
    let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);

    let mut format = None;
    let mut rest = &bytes[12..];
    while rest.len() >= 8 {
        let (id, size) = (&rest[0..4], u32_at(rest, 4) as usize);
        let body = &rest[8..];
        // Some writers leave the data size at 0 or too big when streaming; take what's there.
        let body = &body[..size.min(body.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                let mut code = u16_at(body, 0);
                if code == EXTENSIBLE && body.len() >= 26 {
                    code = u16_at(body, 24); // first bytes of the sub-format GUID
                }
                format = Some((code, u16_at(body, 2), u32_at(body, 4), u16_at(body, 12)));
            }
            b"data" => {
                let (code, channels, sample_rate, block_align) =
                    format.context("data chunk before fmt chunk")?;
                if channels == 0 || block_align == 0 || block_align % channels != 0 {
                    bail!("bad channel layout");
                }
                let width = usize::from(block_align / channels);
                let samples = decode(code, width, body)?;
                return Ok(Wav {
                    samples,
                    channels,
                    sample_rate,
                });
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        let advance = 8 + size + (size & 1);
        rest = rest.get(advance..).unwrap_or(&[]);
    }
    bail!("no audio data")
}

fn decode(code: u16, width: usize, data: &[u8]) -> Result<Vec<f32>> {
    let frames = data.chunks_exact(width);
    Ok(match (code, width) {
        (PCM, 1) => frames.map(|s| (f32::from(s[0]) - 128.0) / 128.0).collect(),
        (PCM, 2..=4) => {
            // Little-endian, left-justified in its container: shift up to the top of an i32.
            let scale = 2f32.powi(31);
            frames
                .map(|s| {
                    let mut word = [0u8; 4];
                    word[4 - width..].copy_from_slice(s);
                    i32::from_le_bytes(word) as f32 / scale
                })
                .collect()
        }
        (FLOAT, 4) => frames
            .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
            .collect(),
        (FLOAT, 8) => frames
            .map(|s| f64::from_le_bytes(s.try_into().unwrap()) as f32)
            .collect(),
        (PCM | FLOAT, _) => bail!("unsupported sample size: {} bits", width * 8),
        _ => bail!("unsupported WAV encoding (format {code}); use PCM or float"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(code: u16, channels: u16, width: u16, data: &[u8]) -> Vec<u8> {
        let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
        out.extend(b"LIST\x03\0\0\0abc\0"); // an odd-sized chunk to skip, with padding
        out.extend(b"fmt \x10\0\0\0");
        out.extend(code.to_le_bytes());
        out.extend(channels.to_le_bytes());
        out.extend(16_000u32.to_le_bytes());
        out.extend((16_000 * u32::from(channels * width)).to_le_bytes());
        out.extend((channels * width).to_le_bytes());
        out.extend((width * 8).to_le_bytes());
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn reads_16_bit_pcm() {
        let data: Vec<u8> = [0i16, 16384, -32768]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let wav = parse(&wav(PCM, 1, 2, &data)).unwrap();
        assert_eq!((wav.channels, wav.sample_rate), (1, 16_000));
        assert_eq!(wav.samples, [0.0, 0.5, -1.0]);
    }

    #[test]
    fn reads_8_and_24_bit_pcm() {
        assert_eq!(
            parse(&wav(PCM, 2, 1, &[128, 0])).unwrap().samples,
            [0.0, -1.0]
        );
        let data = [0x00, 0x00, 0x40, 0x00, 0x00, 0xc0]; // 0.5, -0.5
        assert_eq!(parse(&wav(PCM, 1, 3, &data)).unwrap().samples, [0.5, -0.5]);
    }

    #[test]
    fn reads_float() {
        let data: Vec<u8> = [0.25f32, -1.0]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        assert_eq!(
            parse(&wav(FLOAT, 1, 4, &data)).unwrap().samples,
            [0.25, -1.0]
        );
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse(b"not a wav file at all").is_err());
        assert!(parse(&wav(2, 1, 2, &[0, 0])).is_err()); // ADPCM
    }
}
