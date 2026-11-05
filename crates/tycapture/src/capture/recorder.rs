use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::capture::audio::{self, Level};
use crate::capture::encoder::{Speed, VideoEncoder};
use crate::capture::options::{Quality, RecordingOptions};
use crate::capture::Region;

const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const STDERR_TAIL_LINES: usize = 5;
const INPUT_QUEUE_SIZE: &str = "1024";
/// The file descriptor ffmpeg reads wf-recorder's frames from (`pipe:3`).
const WAYLAND_VIDEO_FD: i32 = 3;

/// Where the video comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoSource {
    /// ffmpeg's x11grab on this X display.
    X11 { display: String },
    /// `wf-recorder` (wlroots screencopy), piped into ffmpeg as raw frames. `origin` is the
    /// layout position of the screen's top-left corner (regions are relative to it).
    Wayland { origin: (i32, i32) },
}

impl VideoSource {
    /// The source for the current session. On Wayland this must run on the GTK thread.
    pub fn current() -> Result<Self> {
        if ty_common::session::is_wayland() {
            let bounds = ty_common::screen::bounds()?;
            Ok(Self::Wayland {
                origin: (bounds.x, bounds.y),
            })
        } else {
            let display = std::env::var("DISPLAY").context("DISPLAY is not set")?;
            Ok(Self::X11 { display })
        }
    }
}

/// An in-progress recording: ffmpeg grabbing the screen (x11grab) or reading frames from
/// wf-recorder. Always written as Matroska, which stays playable if tycapture or ffmpeg dies
/// mid-recording; `capture::transcode` produces the final file.
pub struct Recording {
    child: Child,
    /// wf-recorder, on Wayland.
    grabber: Option<Child>,
    output: PathBuf,
    region: Region,
    started: Instant,
    mic_level: Option<Level>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

/// Resolved audio inputs for one recording.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioInputs {
    microphone: Option<String>,
    system: Option<String>,
}

impl Recording {
    pub fn start(
        ffmpeg: &str,
        source: &VideoSource,
        region: Region,
        options: &RecordingOptions,
        encoder: &VideoEncoder,
        output: PathBuf,
    ) -> Result<Self> {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory: {}", parent.display())
            })?;
        }

        let inputs = AudioInputs {
            microphone: options.microphone.clone(),
            system: options.system_audio.then(audio::default_monitor_source),
        };
        let metered = inputs.microphone.is_some();

        // On Wayland, wf-recorder grabs the frames and ffmpeg reads them on fd 3.
        let mut command = Command::new(ffmpeg);
        let (mut grabber, frames) = match source {
            VideoSource::Wayland { origin } => {
                let (child, frames) = start_wf_recorder(region, *origin, options.framerate)?;
                let fd = frames.as_raw_fd();
                // SAFETY: dup2 is async-signal-safe, and `frames` stays open until after the
                // spawn below.
                unsafe {
                    command.pre_exec(move || {
                        let result = if fd == WAYLAND_VIDEO_FD {
                            // dup2 onto itself would keep close-on-exec set.
                            let flags = libc::fcntl(fd, libc::F_GETFD);
                            libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC)
                        } else {
                            libc::dup2(fd, WAYLAND_VIDEO_FD)
                        };
                        if result < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                (Some(child), Some(frames))
            }
            VideoSource::X11 { .. } => (None, None),
        };

        let spawned = command
            .args(ffmpeg_args(
                source, region, options, encoder, &inputs, &output,
            ))
            .stdin(Stdio::piped())
            .stdout(if metered {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to launch {ffmpeg}"));
        // ffmpeg has its own copy now; only it should hold the pipe.
        drop(frames);
        let mut child = match spawned {
            Ok(child) => child,
            Err(err) => {
                if let Some(grabber) = grabber.as_mut() {
                    let _ = grabber.kill();
                    let _ = grabber.wait();
                }
                return Err(err);
            }
        };

        let mic_level = child.stdout.take().map(audio::spawn_level_reader);
        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(stderr) = child.stderr.take() {
            let tail = stderr_tail.clone();
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    eprintln!("ffmpeg: {line}");
                    let mut tail = tail.lock().unwrap();
                    if tail.len() == STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            });
        }

        Ok(Self {
            child,
            grabber,
            output,
            region,
            started: Instant::now(),
            mic_level,
            stderr_tail,
        })
    }

    pub fn output(&self) -> &Path {
        &self.output
    }

    pub fn region(&self) -> Region {
        self.region
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Microphone level, if the microphone is being recorded.
    pub fn mic_level(&self) -> Option<Level> {
        self.mic_level.clone()
    }

    /// Returns the exit status if ffmpeg (or wf-recorder) has already exited, for example on
    /// error.
    pub fn try_exit_status(&mut self) -> Option<ExitStatus> {
        if let Some(status) = self.child.try_wait().ok().flatten() {
            return Some(status);
        }
        self.grabber
            .as_mut()
            .and_then(|grabber| grabber.try_wait().ok().flatten())
    }

    /// The last few lines ffmpeg wrote to stderr, for error messages.
    pub fn error_output(&self) -> String {
        let tail = self.stderr_tail.lock().unwrap();
        tail.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    /// Ask ffmpeg to finish the file cleanly, killing it if it does not exit in time.
    pub fn stop(mut self) -> Result<PathBuf> {
        if self.grabber.is_some() {
            // Reading frames from a pipe, ffmpeg ignores `q`; SIGINT finalizes the file too.
            // SAFETY: plain kill(2) on our own child.
            unsafe {
                libc::kill(self.child.id() as libc::pid_t, libc::SIGINT);
            }
        } else if let Some(mut stdin) = self.child.stdin.take() {
            // `q` on stdin makes ffmpeg flush and finalize the container.
            let _ = stdin.write_all(b"q");
        }

        let deadline = Instant::now() + STOP_TIMEOUT;
        let status = loop {
            if let Some(status) = self.child.try_wait()? {
                break Some(status);
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break None;
            }
            thread::sleep(Duration::from_millis(50));
        };
        if let Some(mut grabber) = self.grabber.take() {
            // Without a reader it would block on the pipe; SIGINT lets it exit cleanly.
            // SAFETY: plain kill(2) on our own child.
            unsafe {
                libc::kill(grabber.id() as libc::pid_t, libc::SIGINT);
            }
            let deadline = Instant::now() + STOP_TIMEOUT;
            while grabber.try_wait()?.is_none() {
                if Instant::now() >= deadline {
                    let _ = grabber.kill();
                    let _ = grabber.wait();
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }

        if !self.output.exists() {
            let details = self.error_output();
            match status {
                Some(status) => bail!("ffmpeg failed ({status}): {details}"),
                None => bail!("ffmpeg did not stop in time and wrote no file: {details}"),
            }
        }
        Ok(self.output)
    }
}

/// Start wf-recorder writing raw frames of `region` to a pipe. Returns the child and the
/// pipe's read end. `-D -r` gives a constant frame rate, like x11grab.
fn start_wf_recorder(
    region: Region,
    (x0, y0): (i32, i32),
    framerate: u32,
) -> Result<(Child, OwnedFd)> {
    let mut child = Command::new("wf-recorder")
        .args(wf_recorder_args(region, (x0, y0), framerate))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to run wf-recorder (needed to record on Wayland)")?;
    let frames = child.stdout.take().context("no stdout from wf-recorder")?;
    Ok((child, OwnedFd::from(frames)))
}

fn wf_recorder_args(region: Region, (x0, y0): (i32, i32), framerate: u32) -> Vec<String> {
    vec![
        "-g".into(),
        format!(
            "{},{} {}x{}",
            region.x + x0,
            region.y + y0,
            region.width,
            region.height
        ),
        "-r".into(),
        framerate.to_string(),
        "-D".into(),
        "-c".into(),
        "rawvideo".into(),
        "-m".into(),
        "nut".into(),
        "-f".into(),
        "pipe:1".into(),
    ]
}

fn ffmpeg_args(
    source: &VideoSource,
    region: Region,
    options: &RecordingOptions,
    encoder: &VideoEncoder,
    inputs: &AudioInputs,
    output: &Path,
) -> Vec<String> {
    // When a compression pass follows, capture losslessly so quality is only lost once.
    let (quality, audio_codec) = if options.compress {
        (Quality::Lossless, "pcm_s16le")
    } else {
        (options.quality, "aac")
    };
    let video = encoder.args(quality, Speed::Realtime);

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-y".into(),
    ];
    args.extend(video.global);
    args.extend(["-thread_queue_size".into(), INPUT_QUEUE_SIZE.into()]);
    match source {
        VideoSource::X11 { display } => args.extend([
            "-f".into(),
            "x11grab".into(),
            "-framerate".into(),
            options.framerate.to_string(),
            "-draw_mouse".into(),
            u8::from(options.draw_mouse).to_string(),
            "-video_size".into(),
            format!("{}x{}", region.width, region.height),
            "-i".into(),
            format!("{display}+{},{}", region.x, region.y),
        ]),
        VideoSource::Wayland { .. } => args.extend([
            "-f".into(),
            "nut".into(),
            "-i".into(),
            format!("pipe:{WAYLAND_VIDEO_FD}"),
        ]),
    }

    // Input 0 is video; audio inputs follow in this order.
    let audio_sources: Vec<&String> = [&inputs.microphone, &inputs.system]
        .into_iter()
        .flatten()
        .collect();
    for source in &audio_sources {
        args.extend([
            "-thread_queue_size".into(),
            INPUT_QUEUE_SIZE.into(),
            "-f".into(),
            "pulse".into(),
            "-i".into(),
            (*source).clone(),
        ]);
    }

    args.extend(["-map".into(), "0:v".into()]);
    if let Some(graph) = audio_graph(inputs, options.mic_cleanup) {
        args.extend([
            "-filter_complex".into(),
            graph,
            "-map".into(),
            "[aout]".into(),
        ]);
    } else if !audio_sources.is_empty() {
        args.extend(["-map".into(), "1:a".into()]);
    }

    // wf-recorder captures at the monitor's scale; keep the video at the region's size (in
    // layout pixels), which the zoom renderer and cursor log rely on.
    let scale = matches!(source, VideoSource::Wayland { .. })
        .then(|| format!("scale={}:{}", region.width, region.height));
    let filters: Vec<String> = scale
        .into_iter()
        .chain(video.filter.map(str::to_string))
        .collect();
    if !filters.is_empty() {
        args.extend(["-vf".into(), filters.join(",")]);
    }
    args.extend(video.codec);
    if !audio_sources.is_empty() {
        args.extend(["-c:a".into(), audio_codec.into()]);
    }
    args.push(output.display().to_string());

    // Second output: the microphone as raw samples on stdout for the level meter.
    if inputs.microphone.is_some() {
        args.extend(["-map".into(), "1:a".into()]);
        args.extend(audio::meter_output_args());
    }
    args
}

/// Filter graph producing `[aout]`, when the audio needs more than a plain map: microphone
/// cleanup and/or mixing the microphone with system audio.
fn audio_graph(inputs: &AudioInputs, mic_cleanup: bool) -> Option<String> {
    let cleanup = mic_cleanup && inputs.microphone.is_some();
    let system = match (&inputs.microphone, &inputs.system) {
        (Some(_), Some(_)) => Some("[2:a]"),
        _ => None,
    };
    let mix = |mic: &str, system: &str| {
        format!("{mic}{system}amix=inputs=2:duration=longest:normalize=0[aout]")
    };
    match (cleanup, system) {
        (false, None) => None,
        (false, Some(system)) => Some(mix("[1:a]", system)),
        (true, None) => Some(format!("[1:a]{}[aout]", audio::MIC_CLEANUP_FILTER)),
        (true, Some(system)) => Some(format!(
            "[1:a]{}[mic];{}",
            audio::MIC_CLEANUP_FILTER,
            mix("[mic]", system)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{ffmpeg_args, wf_recorder_args, AudioInputs, VideoSource};
    use crate::capture::encoder::VideoEncoder;
    use crate::capture::options::{Format, Quality, RecordingOptions};
    use crate::capture::Region;
    use std::path::Path;

    fn options() -> RecordingOptions {
        RecordingOptions {
            base_name: "test".into(),
            framerate: 30,
            draw_mouse: true,
            format: Format::Mkv,
            quality: Quality::High,
            countdown: 0,
            microphone: None,
            mic_cleanup: false,
            system_audio: false,
            compress: false,
            keep_capture: true,
            window_size: None,
            intro: false,
            intro_title: String::new(),
            intro_logo: None,
            zoom: false,
            frame: Default::default(),
        }
    }

    fn x11() -> VideoSource {
        VideoSource::X11 {
            display: ":0".into(),
        }
    }

    fn args(inputs: AudioInputs) -> String {
        ffmpeg_args(
            &x11(),
            Region::new(100, 50, 1920, 1080),
            &options(),
            &VideoEncoder::Software,
            &inputs,
            Path::new("/tmp/out.mkv"),
        )
        .join(" ")
    }

    #[test]
    fn video_only() {
        let args = args(AudioInputs {
            microphone: None,
            system: None,
        });
        assert!(args.contains("-f x11grab -framerate 30 -draw_mouse 1"));
        assert!(args.contains("-video_size 1920x1080 -i :0+100,50"));
        assert!(args.contains("-crf 18"));
        assert!(!args.contains("pulse"));
        assert!(args.ends_with("/tmp/out.mkv"));
    }

    #[test]
    fn microphone_is_recorded_and_metered() {
        let args = args(AudioInputs {
            microphone: Some("mic".into()),
            system: None,
        });
        assert!(args.contains("-f pulse -i mic -map 0:v -map 1:a"));
        assert!(args.contains("-c:a aac"));
    }

    #[test]
    fn captures_losslessly_when_compressing() {
        let mut options = options();
        options.compress = true;
        let args = ffmpeg_args(
            &x11(),
            Region::new(0, 0, 640, 480),
            &options,
            &VideoEncoder::Software,
            &AudioInputs {
                microphone: Some("mic".into()),
                system: None,
            },
            Path::new("/tmp/out.mkv"),
        )
        .join(" ");
        assert!(args.contains("-preset ultrafast -qp 0"));
        assert!(args.contains("-c:a pcm_s16le"));
        assert!(args.ends_with("/tmp/out.mkv -map 1:a -ac 1 -ar 16000 -f s16le pipe:1"));
    }

    #[test]
    fn microphone_and_system_audio_are_mixed() {
        let args = args(AudioInputs {
            microphone: Some("mic".into()),
            system: Some("sink.monitor".into()),
        });
        assert!(args.contains("-i mic -thread_queue_size 1024 -f pulse -i sink.monitor"));
        assert!(args.contains("amix=inputs=2"));
        assert!(args.contains("-map [aout]"));
    }

    #[test]
    fn cleans_up_microphone_before_mixing() {
        let mut options = options();
        options.mic_cleanup = true;
        let inputs = AudioInputs {
            microphone: Some("mic".into()),
            system: Some("sink.monitor".into()),
        };
        let args = ffmpeg_args(
            &x11(),
            Region::new(0, 0, 640, 480),
            &options,
            &VideoEncoder::Software,
            &inputs,
            Path::new("/tmp/out.mkv"),
        )
        .join(" ");
        assert!(args.contains("-filter_complex [1:a]highpass"));
        assert!(args.contains("[mic];[mic][2:a]amix=inputs=2"));
        // The meter still reads the raw microphone.
        assert!(args.ends_with("-map 1:a -ac 1 -ar 16000 -f s16le pipe:1"));
    }

    #[test]
    fn hardware_encoder_uploads_frames() {
        let args = ffmpeg_args(
            &x11(),
            Region::new(0, 0, 640, 480),
            &options(),
            &VideoEncoder::Vaapi {
                device: "/dev/dri/renderD128".into(),
            },
            &AudioInputs {
                microphone: None,
                system: None,
            },
            Path::new("/tmp/out.mkv"),
        )
        .join(" ");
        assert!(args.starts_with("-hide_banner -loglevel error -nostats -y -vaapi_device"));
        assert!(args.contains("-map 0:v -vf format=nv12,hwupload -c:v h264_vaapi"));
    }

    #[test]
    fn wayland_reads_frames_from_wf_recorder() {
        let args = ffmpeg_args(
            &VideoSource::Wayland { origin: (0, 0) },
            Region::new(10, 20, 640, 480),
            &options(),
            &VideoEncoder::Vaapi {
                device: "/dev/dri/renderD128".into(),
            },
            &AudioInputs {
                microphone: None,
                system: None,
            },
            Path::new("/tmp/out.mkv"),
        )
        .join(" ");
        assert!(args.contains("-thread_queue_size 1024 -f nut -i pipe:3"));
        assert!(!args.contains("x11grab"));
        assert!(args.contains("-vf scale=640:480,format=nv12,hwupload"));
    }

    #[test]
    fn wf_recorder_geometry_is_absolute() {
        let args = wf_recorder_args(Region::new(10, 20, 640, 480), (-1920, 0), 30).join(" ");
        assert_eq!(
            args,
            "-g -1910,20 640x480 -r 30 -D -c rawvideo -m nut -f pipe:1"
        );
    }
}
