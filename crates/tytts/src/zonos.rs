use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::Engine as _;

use crate::native::{command, Process};
use crate::{Asset, Config};

pub const REVISION: &str = "d2d142a86b1e6aeb4252679673c86ecd86177bf8";
const BASE: &str = "https://huggingface.co/Zyphra/ZONOS3-Mini/resolve/d2d142a86b1e6aeb4252679673c86ecd86177bf8/gguf";
const ASSETS: &[Asset] = &[
    Asset {
        local: "backbone-ptq.gguf",
        remote: "backbone-ptq.gguf",
        bytes: 663150624,
        checksum: "66fceb6d73a8c30bde80249e4112bb308380ab7f3384fa11c8e4048115e4ceec",
    },
    Asset {
        local: "zac-keep2.gguf",
        remote: "zac-keep2.gguf",
        bytes: 452325440,
        checksum: "834e5b70b88294afc3c702a478487e0664dcc6b574c16fc6167d72444aabeafd",
    },
    Asset {
        local: "spk-encoder.gguf",
        remote: "spk-encoder.gguf",
        bytes: 45385728,
        checksum: "9539f340b70be246005403b9c254732e180e85e0b3c57fb1973f0d6bc6c59bdc",
    },
    Asset {
        local: "savannah.npy",
        remote: "voices/English/Savannah_en-American_F_1.embedding.npy",
        bytes: 896,
        checksum: "dd81587e66f96fb62da47ef13d5d867de464ca386a83f01b4403cdd19629ca6c",
    },
];

pub fn model(config: &Config, clone: bool, default_voice: bool, quiet: bool) -> Result<PathBuf> {
    let custom = config
        .model
        .as_ref()
        .map(|p| ty_common::config::expand_user(p));
    let dir = custom
        .clone()
        .unwrap_or(crate::model_root(config)?.join(format!("zonos3-mini-{REVISION}")));
    for (i, asset) in ASSETS.iter().enumerate() {
        if (clone && i != 2) || (!clone && (i == 2 || (i == 3 && !default_voice))) {
            continue;
        }
        if custom.is_some() {
            if !dir.join(asset.local).is_file() {
                bail!("Zonos bundle missing {}", dir.join(asset.local).display());
            }
        } else {
            crate::download(&dir, BASE, asset, config.auto_download, quiet)?;
        }
    }
    if !clone {
        crate::laci::prepare(config, &dir, quiet)?;
    }
    Ok(dir)
}

pub fn clone_voice(dir: &Path, input: &Path, output: &Path, gpu: bool) -> Result<()> {
    let mut child = Process(
        command("spk-encoder-cli")
            .arg(dir.join("spk-encoder.gguf"))
            .arg("--audio")
            .arg(input)
            .arg(output)
            .arg(if gpu { "--gpu" } else { "--cpu" })
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context(
                "starting spk-encoder-cli; install the Zonos native runtime or use the Nix package",
            )?,
    );
    let stderr = child
        .0
        .stderr
        .take()
        .context("opening encoder diagnostics")?;
    // Continuously drain diagnostics, retaining only a bounded tail.
    let tail = Arc::new(Mutex::new(String::new()));
    let log = tail.clone();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            append_log(&log, &line);
        }
    });
    let status = child.0.wait()?;
    let _ = reader.join();
    if !status.success() {
        bail!(
            "Zonos voice encoding failed: {status}\n{}",
            tail.lock().unwrap()
        );
    }
    if !output.is_file() {
        bail!("Zonos encoder produced no embedding");
    }
    Ok(())
}

fn append_log(log: &Mutex<String>, line: &str) {
    let mut log = log.lock().unwrap();
    if log.len() > 8192 {
        log.clear();
    }
    log.push_str(line);
    log.push('\n');
}

pub struct Server {
    process: Process,
    url: String,
    token: String,
    agent: ureq::Agent,
    log: Arc<Mutex<String>>,
}

impl Server {
    pub fn start(dir: &Path, gpu: bool, language: &str) -> Result<Self> {
        let mut random = [0u8; 32];
        fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let token = base64::engine::general_purpose::STANDARD.encode(random);
        let language = crate::laci::language(language);
        let heads = crate::laci::path(dir, &language)?;
        let mut process = Process(command("zonos3-server")
            .arg(dir.join("backbone-ptq.gguf"))
            .args(["--host", "127.0.0.1", "--port", "0"])
            .arg("--laci-heads").arg(format!("{language}={}", heads.display()))
            .arg("--zac").arg(dir.join("zac-keep2.gguf"))
            .arg(if gpu { "--gpu" } else { "--cpu" })
            .env("ZONOS3_ACCESS_TOKEN", &token)
            .stdout(Stdio::null()).stderr(Stdio::piped())
            .spawn().context("starting zonos3-server; install the Zonos native runtime or use the Nix package")?);
        let stderr = process
            .0
            .stderr
            .take()
            .context("opening Zonos diagnostics")?;
        let (tx, rx) = mpsc::sync_channel(1);
        let log = Arc::new(Mutex::new(String::new()));
        let tail = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(rest) =
                    line.strip_prefix("zonos3-server: listening on http://127.0.0.1:")
                {
                    if let Some(port) = rest
                        .split_whitespace()
                        .next()
                        .and_then(|p| p.parse::<u16>().ok())
                    {
                        let _ = tx.try_send(port);
                    }
                }
                append_log(&tail, &line);
            }
        });
        let port = rx.recv_timeout(Duration::from_secs(300)).with_context(|| {
            format!(
                "Zonos did not start within five minutes:\n{}",
                log.lock().unwrap()
            )
        })?;
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(300)))
            .timeout_recv_body(Some(Duration::from_secs(1800)))
            .build()
            .into();
        Ok(Self {
            process,
            url: format!("http://127.0.0.1:{port}"),
            token,
            agent,
            log,
        })
    }

    pub fn stream(&self, text: &str, voice: &Path, language: &str) -> Result<Pcm> {
        let mut embedding = Vec::new();
        fs::File::open(voice)
            .context("reading Zonos speaker embedding")?
            .take(65537)
            .read_to_end(&mut embedding)?;
        if embedding.len() > 65536 {
            bail!("Zonos expects a small .npy speaker embedding");
        }
        let request = serde_json::json!({
            "text": text, "language": crate::laci::language(language), "stream": true,
            // Hold audio until alignment retries and short-prompt trimming finish.
            "laci": true, "short_text": "auto", "stream_decode": "whole",
            "speaker_mode": "embedding",
            "speaker_embedding_base64": base64::engine::general_purpose::STANDARD.encode(embedding),
        });
        let mut response = self
            .agent
            .post(format!("{}/v2/generate", self.url))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send(serde_json::to_vec(&request)?)
            .context("requesting Zonos synthesis")?;
        if !response.status().is_success() {
            let status = response.status();
            let mut message = String::new();
            response
                .body_mut()
                .as_reader()
                .take(8192)
                .read_to_string(&mut message)?;
            bail!("Zonos synthesis failed ({status}): {message}");
        }
        Ok(Pcm {
            reader: Box::new(response.into_body().into_reader()),
            pending: Vec::new(),
            done: false,
        })
    }

    pub fn check(&mut self) -> Result<()> {
        if let Some(status) = self.process.0.try_wait()? {
            bail!("Zonos exited: {status}\n{}", self.log.lock().unwrap());
        }
        Ok(())
    }
}

pub struct Pcm {
    reader: Box<dyn Read>,
    pending: Vec<u8>,
    done: bool,
}

impl Iterator for Pcm {
    type Item = Result<Vec<f32>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            let mut bytes = [0u8; 8192];
            match self.reader.read(&mut bytes) {
                Ok(0) => {
                    self.done = true;
                    return if self.pending.is_empty() {
                        None
                    } else {
                        Some(Err(anyhow::anyhow!("truncated Zonos PCM sample")))
                    };
                }
                Ok(n) => self.pending.extend_from_slice(&bytes[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    self.done = true;
                    return Some(Err(e.into()));
                }
            }
            let count = self.pending.len() / 4 * 4;
            if count == 0 {
                continue;
            }
            let samples: Vec<_> = self.pending[..count]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            self.pending.drain(..count);
            if samples.iter().any(|s| !s.is_finite()) {
                self.done = true;
                return Some(Err(anyhow::anyhow!("Zonos returned non-finite audio")));
            }
            return Some(Ok(samples));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_preserves_samples_across_arbitrary_transport_boundaries() {
        struct Fragmented(std::io::Cursor<Vec<u8>>);
        impl Read for Fragmented {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = buf.len().min(3);
                self.0.read(&mut buf[..n])
            }
        }
        let bytes = [0.0f32, -0.5, 0.25, 1.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let mut pcm = Pcm {
            reader: Box::new(Fragmented(std::io::Cursor::new(bytes))),
            pending: Vec::new(),
            done: false,
        };
        assert_eq!(pcm.next().unwrap().unwrap(), [0.0]);
        let rest: Vec<_> = pcm.map(Result::unwrap).flatten().collect();
        assert_eq!(rest, [-0.5, 0.25, 1.0]);
    }

    #[test]
    fn pcm_rejects_truncated_and_invalid_audio() {
        for bytes in [vec![0, 0, 0], f32::NAN.to_le_bytes().to_vec()] {
            let mut pcm = Pcm {
                reader: Box::new(std::io::Cursor::new(bytes)),
                pending: Vec::new(),
                done: false,
            };
            assert!(pcm.next().unwrap().is_err());
            assert!(pcm.next().is_none());
        }
    }
}
