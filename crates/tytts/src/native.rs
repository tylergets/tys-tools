use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use anyhow::{bail, Context, Result};

pub struct Process(pub Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    let parent = std::process::id() as libc::pid_t;
    // SAFETY: only async-signal-safe libc calls between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
    cmd.stdin(Stdio::null());
    cmd
}

pub fn run(cmd: &mut Command) -> Result<()> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = Process(
        cmd.stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!("starting {program}; install the native runtime or use the Nix package")
            })?,
    );
    let stderr = child
        .0
        .stderr
        .take()
        .context("opening helper diagnostics")?;
    let reader = std::thread::spawn(move || {
        let mut tail = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tail.len() > 8192 {
                tail.clear();
            }
            tail.push_str(&line);
            tail.push('\n');
        }
        tail
    });
    let status = child.0.wait()?;
    let tail = reader.join().unwrap_or_default();
    if !status.success() {
        bail!("{program} failed ({status}):\n{tail}");
    }
    Ok(())
}

pub fn run_text(cmd: &mut Command, text: &str) -> Result<()> {
    // A file-backed stdin avoids pipe deadlocks while the helper emits diagnostics.
    let (input, mut file) = crate::Temporary::create(&std::env::temp_dir(), "tts-input.txt")?;
    file.write_all(text.as_bytes())?;
    drop(file);
    cmd.stdin(std::fs::File::open(&input.0)?);
    run(cmd)
}

pub fn wav_stream(file: crate::Temporary) -> Result<(u32, crate::engine::AudioStream)> {
    let reader = hound::WavReader::open(&file.0).context("opening native speech output")?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
        || spec.sample_rate == 0
    {
        bail!("native runtime returned unsupported WAV format");
    }
    Ok((
        spec.sample_rate,
        Box::new(WavBlocks {
            reader,
            _file: file,
            done: false,
        }),
    ))
}

struct WavBlocks {
    reader: hound::WavReader<BufReader<std::fs::File>>,
    _file: crate::Temporary,
    done: bool,
}

impl Iterator for WavBlocks {
    type Item = Result<Vec<f32>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let block = self
            .reader
            .samples::<i16>()
            .take(8192)
            .map(|s| s.map(|s| s as f32 / 32768.0))
            .collect::<Result<Vec<_>, _>>();
        match block {
            Ok(samples) if samples.is_empty() => {
                self.done = true;
                None
            }
            Ok(samples) => Some(Ok(samples)),
            Err(e) => {
                self.done = true;
                Some(Err(e.into()))
            }
        }
    }
}
