# tyspeak

`tyspeak` is a speech-to-text dictation daemon that lives in your system tray, built for i3.
Press a key, talk, press it again, and the text is typed into the focused window. Everything
runs locally.

It uses the same models as [Hex](https://github.com/anomalyco/hex), pinned to the same
Hugging Face revisions and checksums. Like Hex, it runs them with
[transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) (ggml).

## Features

- Tray applet (libappindicator, same as tyboard and tycapture): microphone icon, red while
  recording, yellow while transcribing, downloading or loading a model. **Middle-click
  toggles dictation** (if your tray host forwards secondary activation).
- **Model picker in the tray.** Choosing a model downloads it, with progress in the menu, then
  loads it. The previous model keeps working until the new one is ready. The choice is
  remembered (`${XDG_STATE_HOME:-~/.local/state}/tyspeak/state.json`).
- Resumable, SHA-256-verified downloads.
- Text is typed into the focused window (xdotool on X11, wtype on Wayland), put on the
  clipboard, or both.
- A small D-Bus API on the session bus for i3 bindings, scripts and bars.
- `status --follow` for polybar, eww and other bars.

## Models

| id                       | size   | languages                                          |
| ------------------------ | ------ | -------------------------------------------------- |
| `parakeet-unified-en`    | 731 MB | English; fastest and most accurate (default)       |
| `parakeet-v2`            | 730 MB | English                                            |
| `parakeet-v3`            | 740 MB | 25 European languages, detected automatically      |
| `whisper-large-v3-turbo` | 886 MB | 99 languages; slower                               |
| `qwen3-asr-0.6b`         | 850 MB | multilingual, detected automatically               |
| `sensevoice-small`       | 253 MB | Chinese, Cantonese, English, Japanese, Korean      |

Models are stored in `${XDG_DATA_HOME:-~/.local/share}/tyspeak/models`. On first start, the
selected model downloads automatically (set `auto_download: false` to turn that off).

## Requirements

- X11 session, or a Wayland compositor with the virtual-keyboard protocol (sway, Hyprland, …)
- PulseAudio or PipeWire with `pipewire-pulse` (`parec` records the microphone)
- `xdotool` (typing on X11) or `wtype` and `wl-copy` (Wayland), `notify-send`; optional:
  `xdg-open`
- An AppIndicator/StatusNotifierItem tray host (eww `systray`, polybar with snixembed, KDE, …)

The Nix package wraps the binary with all runtime tools on `PATH`. CPU inference is built for
x86-64-v3 (AVX2/FMA). Parakeet transcribes at about 20× realtime on a desktop CPU.

## Quick Start (Nix)

```bash
nix develop
cargo run -p tyspeak -- run
```

No config file is required; defaults are used if none is found.

## i3

```
# Toggle: press to start, press again to type the text.
bindsym $mod+d exec --no-startup-id tyspeak toggle

# Push to talk: hold to record, release to type.
bindsym Mod1+space exec --no-startup-id tyspeak start
bindsym --release Mod1+space exec --no-startup-id tyspeak stop

bindsym $mod+Escape exec --no-startup-id tyspeak cancel
```

Holding a key makes X send release/press pairs (autorepeat). `stop` waits `audio.tail_ms`
before it takes effect, and a `start` in that window keeps the recording going, so
push-to-talk works while the key is held.

The text is typed with `xdotool type --clearmodifiers`, so modifiers still held from the
binding don't change it. On Wayland, `wtype`'s virtual keyboard has its own modifier state,
so held modifiers don't apply either.

## CLI

```bash
tyspeak run                  # run the daemon (default command)
tyspeak start | stop | toggle | cancel
tyspeak status [--format plain|json|polybar] [--follow]
tyspeak models               # list models (* loaded, > selected)
tyspeak model <id>           # select a model; downloads it if needed
tyspeak transcribe FILE.wav [--model <id>]   # offline, without the daemon
tyspeak validate-config
```

`status --follow` prints a line on every change. While the daemon isn't running it reports
`offline`, which makes it suitable as a bar module:

```ini
; polybar
[module/tyspeak]
type = custom/script
exec = tyspeak status --follow --format polybar
tail = true
click-left = tyspeak toggle
```

For eww, use `deflisten` with `--format json`.

## D-Bus API

The daemon owns `io.github.tylergets.Tyspeak` on the session bus. Owning that name also
keeps a second instance from starting. The object path is `/io/github/tylergets/Tyspeak`,
and the interface is `io.github.tylergets.Tyspeak`.

| Member                         | Description                                                   |
| ------------------------------ | ------------------------------------------------------------- |
| `Start()`                      | Start recording                                               |
| `Stop()`                       | Stop recording, then transcribe and type/copy the text        |
| `Toggle()`                     | Stop if recording, otherwise start                            |
| `Cancel()`                     | Stop recording and discard the audio                          |
| `Status() → s`                 | Status as JSON (see below)                                    |
| `ListModels() → s`             | JSON array of `{id, name, summary, size_bytes, downloaded, selected, active}` |
| `SetModel(s id)`               | Select a model; it is downloaded if needed, then loaded       |
| signal `StatusChanged(s json)` | Emitted on every status change                                |
| signal `Transcribed(s text)`   | Emitted with each finished transcription                      |

Errors come back as `io.github.tylergets.Tyspeak.Error.Failed`, for example when starting
before any model is downloaded.

```bash
dbus-send --session --type=method_call --dest=io.github.tylergets.Tyspeak \
  /io/github/tylergets/Tyspeak io.github.tylergets.Tyspeak.Toggle

# React to transcriptions (for example, with output.mode: none)
dbus-monitor --session "type='signal',interface='io.github.tylergets.Tyspeak',member='Transcribed'"
```

Status JSON fields:

- `state`: `offline|idle|recording|transcribing`
- `model`: the selected model's id
- `model_state`: `missing|downloading|loading|ready|failed`
- `download`: `{model, done_bytes, total_bytes}`, present while a download runs
- `elapsed_secs`
- `mic_level`: 0–1, present while recording

## Configuration

Config is YAML. Discovery order: `./tyspeak.yaml`, then `~/.config/tyspeak/config.yaml`;
override with `--config PATH`. All keys are optional.

```yaml
model: parakeet-unified-en   # used until a model is picked in the tray (that choice wins)
model_dir: null              # default: ~/.local/share/tyspeak/models; supports ~/
auto_download: true          # download the selected model on startup if missing
language: null               # e.g. en, de; passed to models that list it, otherwise ignored
threads: 0                   # CPU inference threads; 0 = automatic
audio:
  source: default            # PulseAudio source (`pactl list short sources`)
  max_duration: 300          # seconds; recording stops by itself after this
  tail_ms: 250               # keep recording this long after stop, so the last word isn't
                             # clipped; a start within this window continues the recording
  start_sound: true          # play a short rising chime when recording starts (via pacat)
  stop_sound: true           # play a short falling chime when recording stops
output:
  mode: type                 # type | clipboard | both | none (signal only)
  trailing_space: true       # so consecutive dictations don't run together
  type_delay_ms: 12          # delay between typed characters
notifications:
  enabled: true              # downloads and errors
```

Recordings shorter than 0.25 s, or quieter than −50 dBFS, are dropped instead of being
transcribed. Set `TYSPEAK_DEBUG=1` to see transcribe.cpp's model and decoder logs.

## Home Manager Module

Part of the [tys-tools](../../README.md) flake:

```nix
{inputs, ...}: {
  imports = [inputs.tys-tools.homeManagerModules.tyspeak];
  programs.tyspeak = {
    enable = true;
    settings = { /* same keys as the YAML config above */ };
  };
}
```

Options: `enable`, `package` (defaults to `pkgs.tyspeak` with the tys-tools overlay), `settings`
(written to `~/.config/tyspeak/config.yaml`), and `service.enable` (systemd user service, default
on).

## Development

From the repo root:

```sh
nix develop
cargo test -p tyspeak
cargo run -p tyspeak -- --help
nix build .#tyspeak
```

Build with `--features vulkan` for GPU inference through ggml's Vulkan backend. This needs
the Vulkan headers, the loader and `glslc`, none of which the flake provides yet.
