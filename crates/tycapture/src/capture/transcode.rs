//! Post-processing of a finished `.capture.mkv` into the final file (intro, frame, compression
//! and remux in a single ffmpeg run), plus an optional zoomed copy next to it (see
//! [`crate::zoom`]).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use ty_common::frame::Frame;

use crate::capture::encoder::{Speed, VideoEncoder};
use crate::capture::options::{Format, Quality};
use crate::zoom::cursor_log::{self, CursorLog};
use crate::zoom::{render, zoomed_path, ZoomSettings};

#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Running { progress: f32 },
    Done { output: PathBuf },
    Failed { error: String },
}

/// A background ffmpeg job. Poll [`TranscodeJob::state`] from the UI.
#[derive(Clone)]
pub struct TranscodeJob {
    pub name: String,
    /// The `.capture.mkv` being processed (removed afterwards unless kept).
    pub capture: PathBuf,
    state: Arc<Mutex<JobState>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscodeSettings {
    pub format: Format,
    pub quality: Quality,
    /// Re-encode with a slow preset; otherwise only remux.
    pub compress: bool,
    pub keep_capture: bool,
    /// Encoder for the compression pass and zoom render. A failing hardware encoder is
    /// retried with libx264.
    pub encoder: VideoEncoder,
    /// Title card prepended to the recording. Forces a re-encode.
    pub intro: Option<IntroSettings>,
    /// Zoom on clicks using the capture's cursor log. Forces a re-encode.
    pub zoom: Option<ZoomSettings>,
    /// Decorative frame around the video (and intro). Forces a re-encode.
    pub frame: Option<FrameSettings>,
}

/// A browser/phone/window frame drawn around the video; the output grows to fit it.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameSettings {
    pub frame: Frame,
    /// Multiplier for the frame's bars and borders.
    pub scale: f64,
    /// RGB (0.0..=1.0) outside the frame's rounded corners.
    pub background: (f64, f64, f64),
    /// Size of the capture the frame goes around.
    pub width: u32,
    pub height: u32,
}

/// A rendered frame image on disk, used as an ffmpeg input.
#[derive(Debug, Clone, PartialEq)]
struct FrameInput {
    path: PathBuf,
    width: u32,
    height: u32,
    left: u32,
    top: u32,
}

/// A black title card with an optional logo and title that fades in and out.
#[derive(Debug, Clone, PartialEq)]
pub struct IntroSettings {
    pub title: String,
    pub logo: Option<PathBuf>,
    pub font: String,
    pub duration: Duration,
    /// Size and frame rate of the capture, which the card must match to be concatenated.
    pub width: u32,
    pub height: u32,
    pub framerate: u32,
    /// Whether the capture has an audio stream (the card then gets silence).
    pub audio: bool,
}

const INTRO_FADE_IN_SECS: f64 = 1.0;
const INTRO_FADE_OUT_SECS: f64 = 0.5;
const INTRO_SAMPLE_RATE: u32 = 48_000;
/// Progress bar ranges when a zoomed copy is made: the plain pass, the zoom render (which does
/// most of the work), then the zoomed copy's pass.
const PLAIN_PROGRESS: (f32, f32) = (0.0, 0.2);
const ZOOM_RENDER_PROGRESS: (f32, f32) = (0.2, 0.85);
const ZOOMED_PROGRESS: (f32, f32) = (0.85, 1.0);

impl TranscodeJob {
    /// Convert `capture` (of length `duration`) into `output` on a background thread.
    pub fn start(
        ffmpeg: &str,
        capture: PathBuf,
        output: PathBuf,
        duration: Duration,
        settings: TranscodeSettings,
    ) -> Self {
        let name = output
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let state = Arc::new(Mutex::new(JobState::Running { progress: 0.0 }));
        let job = Self {
            name,
            capture: capture.clone(),
            state: state.clone(),
        };

        let ffmpeg = ffmpeg.to_string();
        thread::spawn(move || {
            let result = run(&ffmpeg, &capture, &output, duration, &settings, &state);
            let final_state = match result {
                Ok(()) => {
                    if !settings.keep_capture {
                        let log = cursor_log::sidecar_path(&capture);
                        for path in [&capture, &log] {
                            match std::fs::remove_file(path) {
                                Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                                    eprintln!("warning: failed to remove {}: {err}", path.display())
                                }
                                _ => {}
                            }
                        }
                    }
                    JobState::Done { output }
                }
                Err(err) => JobState::Failed {
                    error: format!("{err:#}"),
                },
            };
            *state.lock().unwrap() = final_state;
        });

        job
    }

    pub fn state(&self) -> JobState {
        self.state.lock().unwrap().clone()
    }
}

fn run(
    ffmpeg: &str,
    capture: &Path,
    output: &Path,
    duration: Duration,
    settings: &TranscodeSettings,
    state: &Mutex<JobState>,
) -> Result<()> {
    match run_passes(ffmpeg, capture, output, duration, settings, state) {
        Err(err) if settings.encoder.is_hardware() => {
            eprintln!(
                "warning: {} encoding failed, retrying with libx264: {err:#}",
                settings.encoder.name()
            );
            let software = TranscodeSettings {
                encoder: VideoEncoder::Software,
                ..settings.clone()
            };
            run_passes(ffmpeg, capture, output, duration, &software, state)
        }
        result => result,
    }
}

fn run_passes(
    ffmpeg: &str,
    capture: &Path,
    output: &Path,
    duration: Duration,
    settings: &TranscodeSettings,
    state: &Mutex<JobState>,
) -> Result<()> {
    let Some(zoom) = &settings.zoom else {
        return run_ffmpeg(
            ffmpeg,
            capture,
            output,
            duration,
            settings,
            state,
            (0.0, 1.0),
        );
    };
    run_ffmpeg(
        ffmpeg,
        capture,
        output,
        duration,
        settings,
        state,
        PLAIN_PROGRESS,
    )?;
    run_zoomed(ffmpeg, capture, output, duration, settings, zoom, state)
        .with_context(|| format!("saved {}, but the zoomed copy failed", output.display()))
}

/// Render a zoomed copy of `capture` and post-process it like the plain one, into
/// `foo.zoom.<ext>` next to `output`.
fn run_zoomed(
    ffmpeg: &str,
    capture: &Path,
    output: &Path,
    duration: Duration,
    settings: &TranscodeSettings,
    zoom: &ZoomSettings,
    state: &Mutex<JobState>,
) -> Result<()> {
    let log = CursorLog::load(&cursor_log::sidecar_path(capture))?;
    let rendered = capture.with_extension("zoom.mkv");
    // Lossless when another encode follows; otherwise this is the final encode.
    let quality = if settings.compress {
        Quality::Lossless
    } else {
        settings.quality
    };
    let (start, end) = ZOOM_RENDER_PROGRESS;
    let result = render::render(
        ffmpeg,
        capture,
        &log,
        &rendered,
        zoom,
        quality,
        &settings.encoder,
        |progress| {
            *state.lock().unwrap() = JobState::Running {
                progress: start + progress * (end - start),
            };
        },
    )
    .and_then(|()| {
        run_ffmpeg(
            ffmpeg,
            &rendered,
            &zoomed_path(output),
            duration,
            settings,
            state,
            ZOOMED_PROGRESS,
        )
    });
    let _ = std::fs::remove_file(&rendered);
    result
}

/// Run the intro/compress/remux ffmpeg pass, reporting progress within `(start, end)`.
fn run_ffmpeg(
    ffmpeg: &str,
    capture: &Path,
    output: &Path,
    duration: Duration,
    settings: &TranscodeSettings,
    state: &Mutex<JobState>,
    (start, end): (f32, f32),
) -> Result<()> {
    let mut duration = duration;
    if let Some(intro) = &settings.intro {
        if let Some(logo) = &intro.logo {
            if !logo.is_file() {
                bail!("intro logo not found: {}", logo.display());
            }
        }
        duration += intro.duration;
    }
    let frame = settings
        .frame
        .as_ref()
        .map(|frame| write_frame_image(frame, output))
        .transpose()?;
    let result = run_transcode(
        ffmpeg,
        capture,
        output,
        duration,
        settings,
        frame.as_ref(),
        state,
        (start, end),
    );
    if let Some(frame) = &frame {
        let _ = std::fs::remove_file(&frame.path);
    }
    result
}

/// Render the frame as raw RGBA next to `output`, for ffmpeg to overlay.
fn write_frame_image(settings: &FrameSettings, output: &Path) -> Result<FrameInput> {
    let overlay = settings.frame.overlay(
        settings.width,
        settings.height,
        settings.scale,
        settings.background,
        2,
    )?;
    let mut path = output.as_os_str().to_owned();
    path.push(".frame.rgba");
    let path = PathBuf::from(path);
    std::fs::write(&path, &overlay.rgba)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(FrameInput {
        path,
        width: overlay.width,
        height: overlay.height,
        left: overlay.left,
        top: overlay.top,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_transcode(
    ffmpeg: &str,
    capture: &Path,
    output: &Path,
    duration: Duration,
    settings: &TranscodeSettings,
    frame: Option<&FrameInput>,
    state: &Mutex<JobState>,
    (start, end): (f32, f32),
) -> Result<()> {
    let mut child = Command::new(ffmpeg)
        .args(transcode_args(capture, output, settings, frame))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to launch {ffmpeg}"))?;

    let stderr = child.stderr.take().context("ffmpeg stderr unavailable")?;
    let stderr_reader = thread::spawn(move || {
        let mut tail = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            eprintln!("ffmpeg: {line}");
            tail = line;
        }
        tail
    });

    let stdout = child.stdout.take().context("ffmpeg stdout unavailable")?;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(progress) = parse_progress(&line, duration) {
            *state.lock().unwrap() = JobState::Running {
                progress: start + progress * (end - start),
            };
        }
    }

    let status = child.wait()?;
    let last_error = stderr_reader.join().unwrap_or_default();
    if !status.success() {
        bail!("ffmpeg failed ({status}): {last_error}");
    }
    Ok(())
}

fn transcode_args(
    capture: &Path,
    output: &Path,
    settings: &TranscodeSettings,
    frame: Option<&FrameInput>,
) -> Vec<String> {
    // The intro and frame have to be encoded, so they imply compression.
    let video = (settings.compress || settings.intro.is_some() || frame.is_some())
        .then(|| settings.encoder.args(settings.quality, Speed::Thorough));

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-y".into(),
    ];
    if let Some(video) = &video {
        args.extend(video.global.iter().cloned());
    }
    args.extend(["-i".into(), capture.display().to_string()]);
    let filter = video.as_ref().and_then(|video| video.filter);
    if settings.intro.is_none() && frame.is_none() {
        args.extend(["-map".into(), "0".into()]);
        if let Some(filter) = filter {
            args.extend(["-vf".into(), filter.into()]);
        }
    } else {
        args.extend(filter_graph_args(settings.intro.as_ref(), frame, filter));
    }

    match video {
        Some(video) => {
            args.extend(video.codec);
            args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
        }
        None => args.extend(["-c".into(), "copy".into()]),
    }

    if settings.format == Format::Mp4 {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.display().to_string());
    args
}

/// Inputs, filter graph and maps for an intro and/or frame applied to input 0, in that order
/// (so the frame goes around the intro too). `encoder_filter` is appended last (hardware
/// upload).
fn filter_graph_args(
    intro: Option<&IntroSettings>,
    frame: Option<&FrameInput>,
    encoder_filter: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    let mut graph = Vec::new();
    let mut video = "0:v".to_string();
    let mut next_input = 1;
    // Each stage writes `[cv]` when only the encoder filter follows, `[v]` when nothing does.
    let last = if encoder_filter.is_some() { "cv" } else { "v" };

    if let Some(intro) = intro {
        let out = if frame.is_some() { "intro" } else { last };
        let (inputs, chain) = intro_graph(intro, out);
        args.extend(inputs);
        graph.push(chain);
        next_input += usize::from(intro.logo.is_some());
        video = out.to_string();
    }
    if let Some(frame) = frame {
        args.extend([
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgba".into(),
            "-video_size".into(),
            format!("{}x{}", frame.width, frame.height),
            "-i".into(),
            frame.path.display().to_string(),
        ]);
        // Pad to the frame's size, then lay the frame (transparent where the video shows)
        // over it; the single frame image repeats for the whole video.
        graph.push(format!(
            "[{video}]pad={}:{}:{}:{}[padded];[padded][{next_input}:v]overlay=0:0:format=auto[{last}]",
            frame.width, frame.height, frame.left, frame.top
        ));
        video = last.to_string();
    }
    if let Some(filter) = encoder_filter {
        graph.push(format!("[{video}]{filter}[v]"));
    }

    args.extend([
        "-filter_complex".into(),
        graph.join(";"),
        "-map".into(),
        "[v]".into(),
    ]);
    match intro {
        Some(intro) if intro.audio => args.extend(["-map".into(), "[a]".into()]),
        Some(_) => {}
        None => args.extend(["-map".into(), "0:a?".into()]),
    }
    args
}

/// Inputs and filter chain that put `intro` in front of input 0, writing the joined video to
/// `[out]` and, with audio, the joined audio to `[a]`.
fn intro_graph(intro: &IntroSettings, out: &str) -> (Vec<String>, String) {
    let IntroSettings {
        width,
        height,
        framerate,
        ..
    } = *intro;
    let secs = intro.duration.as_secs_f64();
    let (w, h) = (f64::from(width), f64::from(height));
    let title = !intro.title.trim().is_empty();

    let mut args = Vec::new();
    let mut chain = format!("color=c=black:s={width}x{height}:r={framerate}:d={secs}");
    if let Some(logo) = &intro.logo {
        args.extend([
            "-loop".into(),
            "1".into(),
            "-framerate".into(),
            framerate.to_string(),
            "-t".into(),
            secs.to_string(),
            "-i".into(),
            logo.display().to_string(),
        ]);
        // Fit the logo in a box, leaving room below it for the title.
        let (box_h, center_y) = if title { (0.35, 0.42) } else { (0.5, 0.5) };
        let box_w = even(w * 0.6);
        let box_h = even(h * box_h);
        chain = format!(
            "[1:v]scale=w={box_w}:h={box_h}:force_original_aspect_ratio=decrease,format=rgba[logo];\
             {chain}[bg];[bg][logo]overlay=x=(W-w)/2:y=H*{center_y}-h/2:format=auto"
        );
    }
    if title {
        let center_y = if intro.logo.is_some() { 0.7 } else { 0.5 };
        let font_size = (h / 12.0).round().max(8.0);
        chain.push_str(&format!(
            ",drawtext=font={}:expansion=none:fontcolor=white:fontsize={font_size}\
             :x=(w-text_w)/2:y=h*{center_y}-text_h/2:text={}",
            escape_filter_value(&intro.font),
            escape_filter_value(intro.title.trim()),
        ));
    }
    let fade_out_start = (secs - INTRO_FADE_OUT_SECS).max(0.0);
    chain.push_str(&format!(
        ",fade=t=in:st=0:d={INTRO_FADE_IN_SECS},fade=t=out:st={fade_out_start}:d={INTRO_FADE_OUT_SECS}\
         ,setsar=1,format=yuv420p[iv];[0:v]setsar=1,format=yuv420p[mv];"
    ));
    let joined = format!("[{out}]");
    if intro.audio {
        chain.push_str(&format!(
            "anullsrc=r={INTRO_SAMPLE_RATE}:cl=stereo:d={secs}[ia];\
             [0:a]aresample={INTRO_SAMPLE_RATE},aformat=channel_layouts=stereo[ma];\
             [iv][ia][mv][ma]concat=n=2:v=1:a=1{joined}[a]"
        ));
    } else {
        chain.push_str(&format!("[iv][mv]concat=n=2:v=1:a=0{joined}"));
    }
    (args, chain)
}

/// Round down to an even number of pixels, as yuv420p needs.
fn even(value: f64) -> u32 {
    ((value as u32) / 2 * 2).max(2)
}

/// Escape `value` for use as a filter option inside a filter graph: once for the option
/// parser and once more for the graph parser.
fn escape_filter_value(value: &str) -> String {
    let escape = |text: &str, special: &[char]| {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            if special.contains(&c) {
                out.push('\\');
            }
            out.push(c);
        }
        out
    };
    escape(
        &escape(value, &['\\', '\'', ':']),
        &['\\', '\'', '[', ']', ',', ';'],
    )
}

/// Parse an `out_time_us=` line from `ffmpeg -progress` into a 0.0..=1.0 fraction.
fn parse_progress(line: &str, duration: Duration) -> Option<f32> {
    let micros: u64 = line.strip_prefix("out_time_us=")?.trim().parse().ok()?;
    let total = duration.as_micros().max(1) as f64;
    Some((micros as f64 / total).clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::{
        escape_filter_value, parse_progress, transcode_args, FrameInput, IntroSettings,
        TranscodeSettings,
    };
    use crate::capture::encoder::VideoEncoder;
    use crate::capture::options::{Format, Quality};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn settings(compress: bool, quality: Quality) -> TranscodeSettings {
        TranscodeSettings {
            format: Format::Mp4,
            quality,
            compress,
            keep_capture: true,
            encoder: VideoEncoder::Software,
            intro: None,
            zoom: None,
            frame: None,
        }
    }

    fn intro(logo: Option<&str>, audio: bool) -> IntroSettings {
        IntroSettings {
            title: "My Demo".into(),
            logo: logo.map(PathBuf::from),
            font: "Sans".into(),
            duration: Duration::from_secs(3),
            width: 1920,
            height: 1080,
            framerate: 30,
            audio,
        }
    }

    #[test]
    fn parses_progress() {
        let duration = Duration::from_secs(10);
        assert_eq!(parse_progress("out_time_us=5000000", duration), Some(0.5));
        assert_eq!(parse_progress("out_time_us=99000000", duration), Some(1.0));
        assert_eq!(parse_progress("out_time_us=N/A", duration), None);
        assert_eq!(parse_progress("frame=10", duration), None);
    }

    #[test]
    fn compresses_with_quality() {
        let args = transcode_args(
            Path::new("a.capture.mkv"),
            Path::new("a.mp4"),
            &settings(true, Quality::Medium),
            None,
        )
        .join(" ");
        assert!(args.contains("-preset slow -crf 23"));
        assert!(args.contains("-movflags +faststart a.mp4"));
    }

    #[test]
    fn lossless_uses_qp_zero() {
        let args = transcode_args(
            Path::new("a.capture.mkv"),
            Path::new("a.mp4"),
            &settings(true, Quality::Lossless),
            None,
        )
        .join(" ");
        assert!(args.contains("-qp 0"));
    }

    #[test]
    fn remuxes_without_compression() {
        let args = transcode_args(
            Path::new("a.capture.mkv"),
            Path::new("a.mp4"),
            &settings(false, Quality::High),
            None,
        )
        .join(" ");
        assert!(args.contains("-c copy"));
        assert!(!args.contains("libx264"));
    }

    #[test]
    fn escapes_filter_values() {
        assert_eq!(escape_filter_value("plain"), "plain");
        assert_eq!(escape_filter_value("a:b"), "a\\\\:b");
        assert_eq!(escape_filter_value("it's"), "it\\\\\\'s");
        assert_eq!(escape_filter_value("[x],y;"), "\\[x\\]\\,y\\;");
    }

    #[test]
    fn intro_forces_encode_and_concats() {
        let mut settings = settings(false, Quality::High);
        settings.intro = Some(intro(Some("/logo.png"), true));
        let args = transcode_args(
            Path::new("a.capture.mkv"),
            Path::new("a.mp4"),
            &settings,
            None,
        );
        let joined = args.join(" ");
        assert!(joined.contains("-loop 1 -framerate 30 -t 3 -i /logo.png"));
        assert!(joined.contains("libx264"));
        assert!(joined.contains("-map [v] -map [a]"));
        assert!(!joined.contains("-map 0 "));
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.contains("color=c=black:s=1920x1080:r=30:d=3"));
        assert!(graph.contains("text=My Demo"));
        assert!(graph.contains("concat=n=2:v=1:a=1"));
    }

    #[test]
    fn intro_without_audio_or_logo() {
        let mut settings = settings(true, Quality::High);
        settings.intro = Some(intro(None, false));
        let args = transcode_args(
            Path::new("a.capture.mkv"),
            Path::new("a.mp4"),
            &settings,
            None,
        );
        let joined = args.join(" ");
        assert!(!joined.contains("-loop"));
        assert!(!joined.contains("[a]"));
        assert!(joined.contains("concat=n=2:v=1:a=0[v]"));
    }

    #[test]
    fn hardware_encoder_with_intro_uploads_joined_video() {
        let mut settings = settings(true, Quality::High);
        settings.encoder = VideoEncoder::Vaapi {
            device: PathBuf::from("/dev/dri/renderD128"),
        };
        settings.intro = Some(intro(None, true));
        let args = transcode_args(
            Path::new("a.capture.mkv"),
            Path::new("a.mp4"),
            &settings,
            None,
        );
        let joined = args.join(" ");
        assert!(joined.contains("-y -vaapi_device /dev/dri/renderD128 -i a.capture.mkv"));
        assert!(joined.contains("h264_vaapi"));
        assert!(!joined.contains("-vf"));
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.ends_with("concat=n=2:v=1:a=1[cv][a];[cv]format=nv12,hwupload[v]"));
    }

    #[test]
    fn hardware_encoder_without_intro_uses_vf() {
        let mut settings = settings(true, Quality::High);
        settings.encoder = VideoEncoder::Vaapi {
            device: PathBuf::from("/dev/dri/renderD128"),
        };
        let joined = transcode_args(
            Path::new("a.capture.mkv"),
            Path::new("a.mp4"),
            &settings,
            None,
        )
        .join(" ");
        assert!(joined.contains("-map 0 -vf format=nv12,hwupload -c:v h264_vaapi"));
    }

    fn frame_input() -> FrameInput {
        FrameInput {
            path: PathBuf::from("/v/a.mp4.frame.rgba"),
            width: 1922,
            height: 1126,
            left: 1,
            top: 44,
        }
    }

    #[test]
    fn frame_pads_and_overlays() {
        let settings = settings(false, Quality::High);
        let frame = frame_input();
        let args = transcode_args(
            Path::new("a.capture.mkv"),
            Path::new("a.mp4"),
            &settings,
            Some(&frame),
        );
        let joined = args.join(" ");
        assert!(joined
            .contains("-f rawvideo -pix_fmt rgba -video_size 1922x1126 -i /v/a.mp4.frame.rgba"));
        assert!(joined.contains("libx264"), "a frame forces an encode");
        assert!(joined.contains("-map [v] -map 0:a?"));
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert_eq!(
            graph,
            "[0:v]pad=1922:1126:1:44[padded];[padded][1:v]overlay=0:0:format=auto[v]"
        );
    }

    #[test]
    fn frame_goes_around_intro_and_before_upload() {
        let mut settings = settings(true, Quality::High);
        settings.encoder = VideoEncoder::Vaapi {
            device: PathBuf::from("/dev/dri/renderD128"),
        };
        settings.intro = Some(intro(Some("/logo.png"), true));
        let frame = frame_input();
        let args = transcode_args(
            Path::new("a.capture.mkv"),
            Path::new("a.mp4"),
            &settings,
            Some(&frame),
        );
        let joined = args.join(" ");
        assert!(joined.contains("-map [v] -map [a]"));
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.contains("concat=n=2:v=1:a=1[intro][a];[intro]pad="));
        // The logo is input 1, so the frame image is input 2.
        assert!(
            graph.contains("[padded][2:v]overlay=0:0:format=auto[cv];[cv]format=nv12,hwupload[v]")
        );
    }

    #[test]
    fn remux_ignores_hardware_encoder() {
        let mut settings = settings(false, Quality::High);
        settings.encoder = VideoEncoder::Nvenc;
        let joined = transcode_args(
            Path::new("a.capture.mkv"),
            Path::new("a.mp4"),
            &settings,
            None,
        )
        .join(" ");
        assert!(joined.contains("-c copy"));
        assert!(!joined.contains("nvenc"));
    }
}
