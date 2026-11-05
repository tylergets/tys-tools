# tycapture

`tycapture` is a screen recorder that lives in your system tray, built for i3 on X11 and
also working on Wayland (sway, Hyprland and other wlroots compositors).

## Features

- Tray applet (libappindicator, same as tyboard): red icon while recording, yellow while processing;
  **middle-click stops** a recording (if your tray host forwards secondary activation)
- **Record Entire Screen**
- **Record Window**: click a window, pick a size (1080p, 720p, 4K, square, vertical, custom…),
  and tycapture moves it to its own i3 (or sway) workspace as a borderless floating window of
  exactly that size. When recording ends the window goes back where it was. With other
  window managers the window is recorded where it is.
- **Record Region**: drag out a rectangle on a dimmed snapshot of the screen
- On-screen marker: a thin border just outside the recorded area during the countdown and
  recording (never part of the video; hidden for full-screen recordings)
- Per-recording setup dialog: file name, frame rate, format (mkv/mp4), quality, cursor,
  countdown, microphone (with source picker, live level meter and optional voice cleanup),
  system audio, intro card, frame. The last choices are remembered across restarts
  (`${XDG_STATE_HOME:-~/.local/state}/tycapture/last-options.json`) and take precedence over
  the config defaults; delete that file to go back to them
- Frames: put the video in a browser window, phone or app window frame (the same frames as
  tyshot). The frame goes around the video (and the intro), so nothing is covered except the
  rounded corners, and the output grows to fit it. Adding a frame re-encodes the recording
- Hardware encoding: NVENC or VAAPI is detected automatically and used for capture,
  compression and zoom renders, with libx264 as fallback
- "Recording saved" notification with actions: Open, Open Zoomed, Show in Folder, Copy (as a
  file for file managers/browsers/chat apps, and as a path), Move to Trash (the recording and
  everything made from it)
- Crash-safe capture: recordings are written as `<name>-<timestamp>.capture.mkv` first, then
  compressed (or remuxed) into the final file in the background with progress in the menu
- Pauses dunst notifications while recording (restored afterwards)
- CLI control and live status for bars (polybar, eww, …)

## Requirements

- X11 session, or a wlroots Wayland compositor (sway, Hyprland, …)
- `ffmpeg` with x11grab and PulseAudio support (`ffmpeg-full` in nixpkgs), `pactl`,
  `notify-send`; optional: `dunstctl`, `xdg-open`
- On Wayland: `wf-recorder` (recording) and `grim` (the region/window picker)

### Wayland notes

- Recording goes through `wf-recorder`, which captures one monitor: **Record Entire Screen**
  records the focused monitor, and regions are clipped to the monitor under their center.
- Videos are in layout (logical) pixels, so HiDPI monitors are scaled down to the region
  size. The cursor is always recorded (`draw_mouse` has no effect).
- **Record Window** picks the window from an overlay (Wayland clients can't grab the
  pointer); this needs sway or Hyprland. Moving the window aside needs sway.
- **Zoom** needs the pointer position, which only Hyprland exposes. Clicks are read from
  `/dev/input` (be in the `input` group), otherwise the zoom follows the pointer only.
- An AppIndicator/StatusNotifierItem tray host (eww `systray`, waybar, KDE, …)

The Nix package wraps the binary with all runtime tools on `PATH`.

## Quick Start (Nix)

```bash
nix develop
cargo run -p tycapture -- run
```

No config file is required; defaults are used if none is found.

## CLI

```bash
tycapture run                      # run the applet (default command)
tycapture start [screen|window|region] [--now]   # --now skips the setup dialog
tycapture stop                     # stop recording / cancel countdown
tycapture toggle [screen|window|region] [--now]
tycapture status [--format plain|json|polybar] [--follow]
tycapture validate-config
```

`status --follow` prints a line on every change and keeps retrying while the applet isn't
running (it reports `offline`), which makes it suitable as a bar module:

```ini
; polybar
[module/tycapture]
type = custom/script
exec = tycapture status --follow --format polybar
tail = true
click-left = tycapture toggle region
```

For eww, use `deflisten` with `--format json`; the JSON has `state`
(`offline|idle|preparing|countdown|recording`), `elapsed_secs`, `mic_level` (0–1),
`width`, `height`, `countdown`, `output` and `processing` (`[{name, progress}]`).

## Configuration

Config is YAML. Discovery order: `./tycapture.yaml`, then
`~/.config/tycapture/config.yaml`; override with `--config PATH`. All keys are optional.
`recording`, `audio`, `intro`, `zoom.enabled` and `frame.style` values are the defaults pre-filled in the setup
dialog the first time; after that the dialog starts from your last recording's choices.

```yaml
recording:
  output_dir: ~/Videos/tycapture   # default: XDG videos dir + /tycapture
  base_name: tycapture
  framerate: 30
  draw_mouse: true
  format: mkv          # mkv | mp4
  quality: high        # low | medium | high | lossless
  countdown: 0         # seconds, 0–60
  compress: true       # capture losslessly, then re-encode at `quality`
  keep_capture: true   # keep the .capture.mkv after post-processing
  ffmpeg: ffmpeg
  encoder: auto        # auto | software | nvenc | vaapi
  vaapi_device: null   # e.g. /dev/dri/renderD128; null = try each render node
audio:
  microphone: false
  microphone_source: null   # PulseAudio source name; null = default input
  mic_cleanup: false        # high-pass, noise removal, speech leveling, limiter
  system_audio: false       # record the default output's monitor
window:
  workspace: tycapture     # i3 workspace used for window recordings
  restore: true             # put the window back when recording ends
notifications:
  pause_dunst: true
intro:
  enabled: false
  title: ""
  logo: null                # image path, supports ~/
  duration: 3.0             # seconds, 1–10
  font: null                # fontconfig pattern; unset follows the shared appearance.font
frame:                      # frame around the video, added when post-processing
  style: none               # none | browser | mobile | window
  background: "#ffffff"     # shown outside the frame's rounded corners
  scale: 1.0                # 0.5–4.0; size of the bars and borders (2.0 for HiDPI)
marker:                     # border around the recorded area while recording
  enabled: true
  color: "#e5484d"
  width: 3                  # pixels, 1–32
zoom:                       # zoom in on clicks and follow the pointer
  enabled: false
  max_scale: 1.8            # 1.0–4.0
  hold: 1.5                 # seconds to stay zoomed after a click
  lead: 0.35                # seconds to start zooming before a click
  merge_gap: 1.0            # clicks closer than this share one zoom
  dead_zone: 0.5            # fraction of the view the pointer moves in before panning
  click_effect: true        # ripple where each click happened
  ignore_last: 2.0          # ignore clicks this close to the end (the stop click)
  sync_offset: 0.0          # seconds; raise if zooms start late
```

`encoder: auto` tries NVENC, then VAAPI, then falls back to libx264; the choice is printed at
startup (`video encoder: …`). A forced hardware encoder that doesn't work also falls back to
libx264, and a hardware compression pass that fails is retried with libx264. VAAPI can't
encode losslessly, so lossless captures (`compress: true`) use libx264 with VAAPI; NVENC
captures losslessly on the GPU.

With zoom on, the pointer is logged to `<name>.capture.cursor.jsonl` next to the capture
while recording, and a zoomed copy is saved as `<name>.zoom.<ext>` beside the normal file.

## Runtime Paths

- Control socket / instance lock: `${XDG_RUNTIME_DIR}/tycapture-<user>.sock`

## Home Manager Module

Part of the [tys-tools](../../README.md) flake:

```nix
{inputs, ...}: {
  imports = [inputs.tys-tools.homeManagerModules.tycapture];
  programs.tycapture = {
    enable = true;
    settings = { /* same keys as the YAML config above */ };
  };
}
```

Options: `enable`, `package` (defaults to `pkgs.tycapture` with the tys-tools overlay), `settings`
(written to `~/.config/tycapture/config.yaml`), and `service.enable` (systemd user service, default
on).

## Development

From the repo root:

```sh
nix develop
cargo test -p tycapture
cargo run -p tycapture -- --help
nix build .#tycapture
```
