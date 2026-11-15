# tytts

Speak text locally with Piper. The first invocation automatically downloads the
English `en_US-lessac-medium` voice (~63 MB) and its configuration. Subsequent runs
work offline; text never leaves your computer. No daemon is needed.

```sh
tytts "Hello, Tyler."
tytts Hello world
printf 'Your build finished.' | tytts
tytts - < article.txt
tytts --speed 1.25 "A little faster."
tytts --output greeting.wav "Save this instead of playing it."
tytts --model ~/voices/voice.onnx "Use another Piper voice."
nix run github:tylergets/tys-tools#tytts -- "Hello!"
```

Text arguments are joined with spaces. Without arguments (or with `-`), tytts reads
stdin to EOF. Whitespace is normalized before synthesis so multiline input is spoken
in full. Empty input is an error. Playback blocks until speech finishes; `--output`
writes a WAV instead, replacing the destination if it exists.

## Configuration

Home Manager:

```nix
imports = [ inputs.tys-tools.homeManagerModules.tytts ];
programs.tytts = {
  enable = true;
  settings = {
    auto_download = true;
    speed = 1.0;
    # model = "~/voices/voice.onnx";
    # model_dir = "~/.local/share/tytts/models";
  };
};
```

Or `~/.config/tytts/config.yaml` (also discovers `./tytts.yaml`; `--config PATH`
overrides discovery):

```yaml
auto_download: true
speed: 1.0
# model: ~/voices/voice.onnx
# model_dir: ~/.local/share/tytts/models
```

`--model` and `--speed` override config. A custom `model` must already exist with its
adjacent `<model>.json` file; only the bundled default voice is auto-downloaded.
`model_dir` controls the default voice's cache, normally
`$XDG_DATA_HOME/tytts/models` (`~/.local/share/tytts/models`). Downloads use a pinned
revision, verify SHA-256 and size, and publish files atomically. A failed download
can be retried by running the command again. Set `auto_download: false` for offline-only
use after fetching the voice. Persist this directory on ephemeral systems.

## Requirements

- `piper` from **piper-tts** for CPU speech synthesis.
- `paplay` from **pulseaudio**, with PulseAudio or PipeWire's PulseAudio compatibility
  server, for playback. Saving WAV files does not require an audio server.
- Internet access for the first default-voice download only.

The Nix package includes the runtime programs. Works on X11, Wayland, and from a
terminal without a display server. Voice weights have their own dataset terms;
see the [Lessac model card](https://huggingface.co/rhasspy/piper-voices/blob/e9ab671f2a0e50a701b34361136ba3e6a3a9eb0c/en/en_US/lessac/medium/MODEL_CARD).
