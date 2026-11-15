# tytts

Speak text locally with **Pocket TTS**, **Zonos3-Mini**, **Kokoro**, **Piper**, or **Qwen3-TTS**.
Pocket is the default; Zonos and Qwen support saved voice clones, while Kokoro and Piper
provide selectable preset voices. Pocket audio streams from its
stateful FlowLM/Mimi pipeline into playback as it is generated; there is no
intermediate WAV or external synthesis/player executable. The first invocation
downloads the English January 2026 model, tokenizer, and Alba voice (~236 MB).
Subsequent runs work offline; text never leaves your computer. No daemon is needed.

All model, voice, alignment-head, and archive downloads are resumable. Interrupted
transfers stay beside the destination as `<filename>.part`; rerun the same command
to continue. The shared `ty-common` downloader also powers tyspeak. It verifies the
full SHA-256 before publishing a file, retries corrupt downloads once from scratch,
and restarts safely if a server ignores byte-range requests. Concurrent invocations
wait on an OS file lock. The small `<filename>.download.lock` files are retained,
but locks are released automatically when a process exits or crashes.

```sh
tytts "Hello, Tyler."
tytts Hello world
printf 'Your build finished.' | tytts
tytts - < article.txt
tytts --speed 1.25 "A little faster."
tytts --output greeting.wav "Save this instead of playing it."
tytts --voice ~/voices/voice.safetensors "Use another compatible voice."
tytts --voices
tytts --engine zonos --clone recording.wav --name tyler
tytts --voice tyler "Hello in my cloned voice."
tytts --voice-info tyler
tytts --remove-voice tyler
tytts --engine zonos --voice savannah --language fr "Bonjour."
tytts --engine kokoro "Hello from Kokoro."
tytts --voice af_heart "Use Kokoro's Heart voice."
tytts --voice bf_emma "Use a British voice."
tytts --voice ff_siwis "Bonjour."
tytts --engine piper "Hello from Piper."
tytts --voice en_US-amy-low "Use Piper's Amy voice."
tytts --voices --engine kokoro
tytts --voices --language fr
tytts --voices --all-languages
tytts --engine qwen "Hello from Qwen."
tytts --voice qwen-aiden --instruct "Speak warmly and naturally." "Welcome home."
tytts --quiet "Hide progress."
nix run github:tylergets/tys-tools#tytts -- "Hello!"
```

Text arguments are joined with spaces. Without arguments (or with `-`), tytts reads
stdin to EOF. Pocket uses **streaming audio output**, not incremental text input:
Pocket TTS receives completed text, then emits audio frames before the utterance
is finished. Model loading and first-frame inference still take time. Long Pocket input
uses the engine's sentence-aware context handling; tytts does not split incoming
text into independent fixed-size synthesis calls. Empty input is an error.

Playback blocks until speech finishes. Pocket buffers the first audio and
re-buffers after generation gaps. Zonos, Kokoro, Piper, and Qwen generate the entire utterance into a
temporary WAV before playback, so slow inference adds a startup wait rather than
stuttering. The temporary file is removed when the command exits normally.
`--output` writes each audio block to a
16-bit mono WAV instead, replacing the destination if it exists. Download progress
bars show bytes, rate, and ETA; spinners identify loading, synthesis, and playback,
with generated audio duration. Progress goes to stderr, uses plain phase messages
when redirected, and can be disabled with `--quiet`.

## Configuration

Home Manager:

```nix
imports = [ inputs.tys-tools.homeManagerModules.tytts ];
programs.tytts = {
  enable = true;
  settings = {
    engine = "pocket"; # or "zonos", "kokoro", "piper", "qwen"
    auto_download = true;
    speed = 1.0;
    # voice = "~/voices/voice.safetensors";
    # model_dir = "~/.local/share/tytts/models";
    # voice = "tyler"; # named voices automatically choose their engine
    # language = "en_us";
    # gpu = false; # force CPU for Zonos/Qwen (GPU is automatic by default)
    # instruct = "Speak warmly."; # Qwen presets only
  };
};
```

Or `~/.config/tytts/config.yaml` (also discovers `./tytts.yaml`; `--config PATH`
overrides discovery):

```yaml
engine: pocket # or zonos, kokoro, piper, qwen
auto_download: true
speed: 1.0
# voice: ~/voices/voice.safetensors
# model_dir: ~/.local/share/tytts/models
# voice: tyler
# language: en_us
# gpu: false # force CPU for Zonos/Qwen (GPU is automatic by default)
# instruct: "Speak warmly." # Qwen presets only
```

`--engine`, `--model`, `--voice`, `--language`, and `--speed` override config.
Zonos automatically selects a GPU when available, with CPU fallback. Set `gpu: false`
in YAML or `gpu = false;` in Nix to force CPU. Qwen also uses this setting;
Pocket, Kokoro, and Piper ignore it.
Named voices select their
engine automatically; an explicitly conflicting `--engine` is an error.
`speed` ranges from 0.25 to 4
(default 1); a continuous Sonic time-stretching stream changes tempo without
changing pitch. At speed 1 the audio bypasses that processor.

`model_dir` controls the download cache root, normally `$XDG_DATA_HOME/tytts/models`
(`~/.local/share/tytts/models`). Pocket TTS assets live in its `pocket-tts-2026-01/`
subdirectory. Downloads use a pinned revision, verify SHA-256 and size, and publish
files atomically. A failed download can be retried by running the command again.
Set `auto_download: false` for offline-only use after fetching the assets. Persist
this directory on ephemeral systems.

A custom Pocket `voice` is a local compatible Pocket TTS **conditioning embedding**
(`.safetensors`), not a WAV recording or an exported KV-cache voice state.
A custom `model` is a local directory containing:

- `model.safetensors`: compatible Pocket TTS weights.
- `tokenizer.model`: its SentencePiece tokenizer.
- `config.json`: its `ptts::tts_model::TTSConfig` architecture configuration.
- `voice.safetensors`: a compatible embedding, unless `voice` / `--voice` overrides it.

Custom bundles and voices are not downloaded automatically. Pocket text normalization
is English. Older Piper settings pointing directly to an `.onnx` file need to be
migrated to `engine: piper` with a sherpa-onnx bundle directory, as described below.

## Voices and cloning

```sh
tytts --voices                     # OS-language presets and saved voices; no downloads
tytts --voices --engine zonos      # filter by engine
tytts --voices --language fr       # override language filter
tytts --voices --all-languages     # every language
tytts --voice-info tyler           # engine, model compatibility, embedding path
tytts --clone sample.mp3 --name tyler
tytts --voice tyler --output preview.wav "A preview of my voice."
tytts --remove-voice tyler         # also removes the retained recording
```

Built-in names include `alba` (Pocket), `savannah` (Zonos), and the Kokoro/Piper/Qwen
presets listed by `--voices`. Built-in names cannot be deleted or used for clones.
The aligned table shows name, engine, language, gender, and kind. Listing uses
`--language`, then configured `language`, then the first nonempty OS locale in
`LC_ALL`, `LC_MESSAGES`, `LANG`. Missing, `C`, and `POSIX` locales default to US English.
Regions are respected: `en_US.UTF-8` excludes British presets; `en-GB` excludes
US presets. Use `--language en` for all English variants. Voices tagged with a
generic language remain visible for that language's regions.
`--all-languages` disables language filtering and can be combined with
`--engine`. Saved clones have no recorded reference language; they remain visible
with language `und` (undetermined). This filters the catalog, not which languages
a multilingual engine can speak. Synthesis keeps its existing engine/voice defaults.
Kokoro prefixes supply language and gender: `af`/`am` are American female/male,
`bf`/`bm` British female/male; `e`, `f`, `h`, `i`, `p`, and `z` identify Spanish,
French, Hindi, Italian, Brazilian Portuguese, and Chinese. Unknown gender is
shown as `und`; saved clone names are not interpreted as Kokoro prefixes.

`--clone` defaults to
Zonos, regardless of the configured speech engine, and accepts the audio formats
supported by FFmpeg. Use a clean recording of one speaker; the encoder uses up to
60 seconds. No transcript or training is required. Cloning saves a reusable speaker
embedding, not a new set of model weights. The bundled Pocket checkpoint has no
audio encoder. Recording-based cloning is supported by Zonos and Qwen.

Names use ASCII letters, digits, underscores and hyphens (1–64 characters).
Existing names are never overwritten. Saved voices live under
`$XDG_DATA_HOME/tytts/voices/NAME/`, with `manifest.json`, `embedding.npy`, and a copy
of the original recording. Completed voices are published atomically. A process
killed during cloning may leave a hidden staging directory and `.NAME.lock`; remove
those only after confirming that no clone operation is running.

Embeddings are engine/model-specific. Saved voices record their model revision;
custom model bundles are identified by their canonical directory path. Keep custom
bundles immutable or recreate their voices when replacing weights. A custom-bundle
voice requires the same `--model` directory on subsequent invocations.

## Zonos runtime

Zonos uses the pinned [native ggml runtime](https://github.com/Zyphra/ZONOS3-Mini/tree/96d9e0ec1e39083ed336ac78f24e3dbc93721c0f/servers/ggml),
without Python. tytts launches a private, authenticated loopback server on an
ephemeral port for each speech invocation, receives 44.1 kHz mono PCM, then stops
the process before playback. Audio is spooled to a temporary WAV for smooth
playback without keeping the whole utterance in memory. `--output` writes directly
to the requested file. No service configuration is needed. The Nix package includes the
runtime and its Vulkan backend; GPU selection is automatic, with upstream CPU fallback.
Outside Nix, put `zonos3-server`, `spk-encoder-cli`, and `ffmpeg` on PATH.

The first Zonos synthesis downloads the quantized backbone and decoder (~1.12 GB)
plus the default voice if needed. Cloning downloads only the speaker encoder
(~45 MB); the synthesis weights are fetched when the saved voice is first spoken.
Assets use pinned revision `d2d142a86b1e6aeb4252679673c86ecd86177bf8`, size/SHA-256
verification, and atomic publication, under `model_dir/zonos3-mini-<revision>/`.
`auto_download: false` also applies to Zonos.

Zonos enables alignment-guarded decoding (LACI) to reduce skipped text and
unrequested words after the prompt. A small, checksum-verified alignment file
downloads for the selected language. One- or two-word prompts use the runtime's
guarded short-text mode, which removes its internal filler before returning audio.
This improves adherence but does not guarantee perfect neural-model output.

Zonos `--model DIR` expects `backbone-ptq.gguf` and `zac-keep2.gguf` for synthesis,
`spk-encoder.gguf` for cloning, and `savannah.npy` if no voice is selected.
`--voice PATH` accepts a compatible 192-dimensional float32 `.npy` speaker
embedding. Zonos weights are Apache-2.0; runtime code is MIT.
Custom bundles also need the matching `zonos3_mini_heads*.json` file from the
release's `web/laci/` directory (`zonos3_mini_heads.json` for English).

`--language` defaults to `en_us` for Zonos; use its language codes such as `fr`,
`de`, or `ja`. The bundled Pocket model remains English-only. Zonos has finite
context and generation limits: very long inputs may need shorter utterances.
All engines read stdin to EOF; none incrementally reads speech text from stdin.
Pocket plays while generating; Zonos starts playback after generation completes.

## Kokoro and Piper

Both use the native `sherpa-onnx-offline-tts` executable (no Python), on CPU.
The helper generates a temporary WAV, exits, and tytts applies the same speed
control and playback/output path used by the other engines. They accept up to
100,000 UTF-8 bytes per request; split longer input into separate requests.

### Kokoro

The default is the full-precision Kokoro v1.0 bundle (~350 MB compressed download), with
`af_heart` as the default voice. Full precision avoids the narrow-band ringing
observed with the int8 export. It uses a separate cache directory, so the next
default Kokoro request downloads the full-precision bundle automatically.
An explicit `--model` still uses the supplied bundle; select a full-precision
bundle if an existing int8 custom model rings.
`--voices --engine kokoro --all-languages` lists 48 presets with
stable names, including `af_bella`, `am_michael`, `bf_emma`, `ff_siwis`, and
`zf_xiaobei`. The voice chooses its native language by default; override with
`--language en`, `en-us`, `en-gb`, `zh`, `es`, `fr`, `hi`, `it`, or `pt-br`.
Japanese voices are omitted because this bundle has no Japanese text frontend.
Kokoro weights are Apache-2.0.

### Piper

The default voice is `en_US-lessac-medium` (22.05 kHz); `en_US-amy-low`
(16 kHz) is also available. Each bundle downloads separately (~67 MB compressed).
The language is fixed by the voice/model. For other languages, use a compatible
local sherpa-onnx Piper bundle. Voice licenses vary; keep the model's license and
model card with custom bundles.

```sh
tytts --voices --engine piper
tytts --voice-info en_US-lessac-medium
tytts --voice en_US-lessac-medium --speed 1.2 --output speech.wav "Hello."
tytts --engine piper --model ~/models/vits-piper-de_DE-thorsten-medium "Guten Tag."
```

### Models and runtime

`--model DIR` selects an extracted sherpa-onnx bundle containing exactly one
`.onnx` file, `tokens.txt`, and `espeak-ng-data/`. Kokoro additionally needs
`voices.bin`, `lexicon-us-en.txt`, `lexicon-gb-en.txt`, and `lexicon-zh.txt`;
custom Kokoro bundles must use the same v1.0 speaker-ID mapping.
Custom Piper bundles use speaker 0. A raw Piper `.onnx` plus `.onnx.json` pair is
not sufficient: use a sherpa-onnx-converted model bundle.

Downloaded archives have pinned sizes/SHA-256 hashes and are extracted into a
staging directory before publication. `model_dir` holds both the retained archives
and extracted bundles; `auto_download: false` prevents missing-archive downloads.
An incomplete extracted bundle produces an error: move it aside to re-extract
from the retained archive. An interrupted process can leave a hidden staging
directory, which can be removed when no extraction is running.

The Nix package includes sherpa-onnx, GNU tar, and bzip2. Outside Nix, install
these and place `sherpa-onnx-offline-tts`, `tar`, and `bzip2` on PATH.

## Qwen3-TTS

`--engine qwen` uses Qwen3-TTS 1.7B CustomVoice with `qwen-aiden` by default.
The native `qwentts.cpp` helper uses Vulkan on AMD/Intel/NVIDIA GPUs and supports
CPU execution (`gpu: false` in YAML or `gpu = false;` in Home Manager settings).
The Nix package includes `qwen-tts`, `qwen-codec`, and FFmpeg; outside Nix put
those executables on PATH. A working Vulkan driver is needed for GPU execution.

The BF16 talker and F32 codec download automatically (~4.5 GB combined), with
pinned revisions and SHA-256 checks. The codec is kept full precision.
`auto_download: false` disables downloads. The models are Apache-2.0, and the
native runtime is MIT.

```sh
tytts --voices --engine qwen
tytts --voices --engine qwen --all-languages
tytts --voice qwen-ryan --output narration.wav "A new chapter begins."
tytts --voice qwen-aiden --instruct "Speak calmly, with a reassuring tone." "Take your time."
tytts --voice qwen-ono-anna --language ja "こんにちは。"
tytts --engine qwen --clone recording.wav --name my-qwen \
  --transcript "The exact words spoken in the recording."
tytts --voice my-qwen "Speak with my saved voice."
```

There are nine presets: `qwen-aiden`, `qwen-ryan`, `qwen-vivian`, `qwen-serena`,
`qwen-uncle-fu`, `qwen-dylan`, `qwen-eric`, `qwen-ono-anna`, and `qwen-sohee`.
Their native languages and genders appear in the voice catalog. Synthesis defaults
to the preset's native language; clones default to English. Override with
`--language en`, `zh`, `ja`, `ko`, `de`, `fr`, `ru`, `pt`, `es`, `it`, or `auto`.
`--instruct` (config: `instruct`) controls preset delivery; it is not supported
by the Base cloning model.

Cloning downloads the separate 1.7B Base talker (~3.9 GB), reusing the codec.
FFmpeg converts the reference to mono 24 kHz; the voice stores its speaker
embedding and acoustic codes. Supply an accurate `--transcript` for in-context
cloning; omitting it uses speaker-embedding-only cloning, which may be less faithful.
Saved Qwen voices use the same listing, inspection, and removal commands as Zonos.
Plain `--clone` still defaults to Zonos.

Models are cached in `models/qwen3-tts-1.7b-b7ee2e8c/`.
`--model DIR` accepts a bundle with `qwen-tokenizer-12hz-F32.gguf` and
`qwen-talker-1.7b-customvoice-BF16.gguf` (presets) or
`qwen-talker-1.7b-base-BF16.gguf` (cloning). Qwen finishes generating a temporary
WAV before playback, then uses the existing speed control and output pipeline.

## Requirements

- Pocket CPU inference is embedded in Rust through [`ptts` / xn](https://github.com/gradium-ai/xn-ptts).
- Zonos: native runtime and FFmpeg for reference decoding, included in the Nix package.
- Qwen: qwentts.cpp native runtime and FFmpeg, included in Nix; Vulkan driver for GPU execution.
- Kokoro/Piper: sherpa-onnx, GNU tar, and bzip2, included in the Nix package.
- The PulseAudio client libraries, with PulseAudio or PipeWire's PulseAudio
  compatibility server, for playback. Saving WAV files does not need an audio server.
- Internet access for the first model download only; no Hugging Face login required.

The Nix package includes the required libraries. Works on X11, Wayland, and from
a terminal without a display server. Build from source with `nix develop` and
`cargo build --release -p tytts`; release optimization matters for inference.

Model and voice assets are distributed separately under CC-BY-4.0 by Kyutai:
see the [Pocket TTS model card](https://huggingface.co/kyutai/pocket-tts-without-voice-cloning)
and [Pocket TTS project](https://github.com/kyutai-labs/pocket-tts).
