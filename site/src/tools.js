// Every tool on the site. The home page and tool pages are all built from this list.
// Keep text short; full details live in each crate's README.

export const repo = "https://github.com/tylergets/tys-tools";
export const license = `${repo}/blob/main/LICENSE`;

export const tools = [
  {
    name: "tytts",
    tagline: "Speak text with a local voice.",
    summary:
      "Read text aloud with Pocket TTS, Zonos3-Mini, Kokoro, Piper, or Qwen3-TTS. Clone recordings with Zonos or Qwen, or choose preset voices. " +
      "Pocket streams audio; the other engines finish generation before playback. Models download on first use, then run offline.",
    features: [
      "Text arguments and piped or redirected stdin",
      "Automatic, checksum-verified model downloads (Pocket ~236 MB; full-precision Kokoro ~350 MB; Piper ~67 MB per voice)",
      "Every download resumes from .part files; shared downloader with tyspeak verifies SHA-256 and serializes concurrent writers",
      "Offline CPU inference after the first download",
      "Embedded Pocket streaming; native helper runtimes for Zonos, Kokoro, and Piper",
      "Pitch-preserving speech speed (0.25–4×) and compatible voice embeddings",
      "Download progress bars and loading/synthesis/playback status; --quiet to hide",
      "Stream to playback or save a WAV; stdin is read to EOF before synthesis",
      "Zonos finishes generation before playback to prevent stuttering on slower CPUs",
      "Zonos text-alignment guard reduces extra words and handles short prompts",
      "Zonos3-Mini multilingual voice cloning: --clone recording.wav --name NAME",
      "Aligned voice table with language and gender parsed from Kokoro prefixes; --voices uses the OS locale",
      "Region-aware filtering: US English hides British presets; --language en includes both; --all-languages shows all",
      "Inspect voices with --voice-info; delete saved clones with --remove-voice",
      "Saved voices select their engine automatically; Zonos supports --language and automatically uses a GPU with CPU fallback",
      "Full-precision Kokoro avoids int8 ringing; 48 presets across seven languages",
      "Piper Lessac and Amy voices; local sherpa-onnx Piper bundles support other languages",
      "Kokoro/Piper CPU inference and up to 100,000 UTF-8 bytes per request",
      "Qwen3-TTS 1.7B: nine presets, voice cloning, and --instruct delivery controls for presets",
      "Qwen BF16 talker and full-precision codec (~4.5 GB); native Vulkan GPU acceleration",
    ],
    wayland: "Works on any desktop, or without a display server.",
    requirements: [
      "PulseAudio client libraries (included in the Nix package)",
      "PulseAudio or PipeWire's PulseAudio server for playback; not needed for WAV output",
      "Internet access for the first model download; no login required",
      "Zonos native server, speaker encoder, and FFmpeg (included in Nix); ~1.12 GB synthesis weights plus ~45 MB cloning encoder",
      "Kokoro/Piper: sherpa-onnx, GNU tar, and bzip2 (included in Nix)",
      "Qwen: qwen-tts, qwen-codec, FFmpeg (included in Nix); Vulkan driver for GPU execution",
    ],
    usage: `tytts "Hello, Tyler."
printf 'Your build finished.' | tytts
tytts - < article.txt
tytts --speed 1.25 "A little faster."
tytts --output greeting.wav "Save this."
tytts --voice ~/voices/voice.safetensors "Another compatible voice."
tytts --voices
tytts --clone recording.wav --name tyler
tytts --voice tyler "My cloned voice."
tytts --voice-info tyler
tytts --remove-voice tyler
tytts --engine kokoro "Hello from Kokoro."
tytts --voice bf_emma "A British voice."
tytts --voice ff_siwis "Bonjour."
tytts --engine piper "Hello from Piper."
tytts --voice en_US-amy-low "Another Piper voice."
tytts --voices --engine kokoro
tytts --voices --engine piper
tytts --voices --language fr
tytts --voices --all-languages
tytts --engine qwen "Hello from Qwen."
tytts --voice qwen-aiden --instruct "Speak warmly." "Welcome home."
tytts --engine qwen --clone recording.wav --name my-qwen --transcript "The exact recorded words."
tytts --voice my-qwen "My Qwen voice."
tytts --quiet "Hide progress."`,
    config: `engine: pocket # or zonos, kokoro, piper, qwen
auto_download: true
speed: 1.0
# voice: ~/voices/voice.safetensors
# voice: af_heart # or en_US-lessac-medium; presets select their engine
# model_dir: ~/.local/share/tytts/models
# language: en_us
# gpu: false # force CPU for Zonos/Qwen; GPU is automatic by default
# instruct: "Speak warmly." # Qwen presets only`,
    nixSettings: `engine = "pocket"; # or "zonos", "kokoro", "piper", "qwen"
auto_download = true;
speed = 1.0;
# voice = "~/voices/voice.safetensors";
# voice = "af_heart"; # or "en_US-lessac-medium"; presets select their engine
# model_dir = "~/.local/share/tytts/models";
# language = "en_us";
# gpu = false; # force CPU for Zonos/Qwen; GPU is automatic by default
# instruct = "Speak warmly."; # Qwen presets only`,
    storage:
      "Assets live in $XDG_DATA_HOME/tytts/models/pocket-tts-2026-01/ (normally under ~/.local/share). " +
      "Interrupted downloads retain .part files; rerun to resume. Persistent .download.lock files carry OS locks released on exit. " +
      "Zonos models use a separate revision-pinned subdirectory. Saved voices and reference recordings live in " +
      "$XDG_DATA_HOME/tytts/voices/; persist both directories. Pocket custom model directories contain model.safetensors, " +
      "tokenizer.model, a ptts config.json, and voice.safetensors (or use --voice). " +
      "Zonos uses GGUF models and .npy speaker embeddings; its private native server runs only during speech. " +
      "Kokoro/Piper archives and extracted bundles also live under models/. Their --model directory needs a " +
      "sherpa-onnx ONNX model, tokens.txt, and espeak-ng-data; Kokoro also needs voices.bin and lexicons. " +
      "Voice filtering uses --language, configured language, then LC_ALL/LC_MESSAGES/LANG (C/POSIX defaults to US English); " +
      "regional variants are filtered separately. Saved clones remain visible with undetermined language and gender. " +
      "Qwen models use models/qwen3-tts-1.7b-b7ee2e8c/: BF16 CustomVoice/Base talkers and an F32 codec. " +
      "Qwen cloning downloads a separate ~3.9 GB Base talker; --transcript enables in-context cloning. " +
      "Zonos and Qwen support recording-based cloning. Kyutai assets are CC-BY-4.0; Zonos, Kokoro, and Qwen weights " +
      "are Apache-2.0; Piper voice licenses vary. See the tool README.",
  },
  {
    name: "tybat",
    tagline: "Battery levels and low-charge warnings.",
    summary:
      "Laptop and connected device batteries in your tray, including Bluetooth devices. " +
      "When the laptop battery is critically low, a focused popup counts down to suspend or hibernate.",
    features: [
      "Tray icon in the shared ty-tools palette and laptop percentage",
      "Optional Framework charge cap with persistent tray overrides and resume reapply",
      "Menu with laptop, Bluetooth and other reported device batteries",
      "UPower discovery with BlueZ Battery1 fallback",
      "Focus-grabbing critical popup with a live countdown",
      "Plugging in cancels; Escape or a button snoozes",
      "Configurable thresholds, countdown, snooze and power action",
      "Safe preview command; peripherals never trigger suspend",
    ],
    wayland: "Exclusive layer-shell keyboard focus on supported compositors; X11 uses an input grab.",
    requirements: [
      "Linux with GTK 3 and UPower on the system D-Bus",
      "A StatusNotifierItem tray for the battery menu",
      "BlueZ and a device reporting battery levels for Bluetooth batteries",
      "Working logind power actions; hibernate requires system configuration",
      "Replace batsignal and keep UPower's emergency threshold below tybat's",
    ],
    usage: `tybat run
tybat status
tybat preview`,
    config: `warning_percent: 20
critical_percent: 5
countdown_seconds: 60
snooze_seconds: 60
poll_seconds: 5
action: suspend
# Optional; use a privileged, restricted helper:
# charge_limit:
#   percent: 80
#   command: [/run/wrappers/bin/sudo, -n, /path/to/charge-limit-helper]`,
    nixSettings: `warning_percent = 20;
critical_percent = 5;
countdown_seconds = 60;
snooze_seconds = 60;
poll_seconds = 5;
action = "suspend";
# Optional; use a privileged, restricted helper:
# charge_limit = {
#   percent = 80;
#   command = [ "/run/wrappers/bin/sudo" "-n" "/path/to/charge-limit-helper" ];
# };`,
  },
  {
    name: "tynotif",
    tagline: "Desktop notifications with history and snooze.",
    summary:
      "An opt-in notification daemon with popup actions, progress, in-memory history " +
      "and tray controls for history and timed snoozing. Works with notify-send and desktop apps.",
    features: [
      "Standard desktop notification D-Bus API",
      "Action buttons, progress bars and urgency styling",
      "Replacement IDs and app-scoped stack tags",
      "Bounded history with JSON output and replay",
      "Bell tray icon with readable notification history",
      "Timed snooze, indefinite pause and instant resume",
      "Do-not-disturb and scriptable dismiss controls",
    ],
    wayland: "Popup surfaces use layer-shell on supported compositors.",
    requirements: [
      "Linux desktop with GTK and a session D-Bus",
      "A StatusNotifierItem tray (optional, for history and sleep controls)",
      "X11 or a Wayland compositor with layer-shell",
      "Stop your current notification daemon before starting tynotif",
    ],
    usage: `tynotif run
tynotif set-paused toggle
tynotif snooze 30
tynotif close
tynotif history
tynotif history-pop
tynotif status`,
    config: `width: 480
limit: 4
timeout_ms: 20000
history_limit: 200`,
    nixSettings: `width = 480;
limit = 4;
timeout_ms = 20000;
history_limit = 200;`,
  },
  {
    name: "tyboard",
    tagline: "Clipboard history in your tray.",
    summary:
      "Remembers the text and images you copy, keeps them across restarts, and puts any " +
      "of them back on the clipboard from the tray menu or a rofi picker.",
    features: [
      "History for text and images",
      "Tray menu with clickable history items",
      "tyboard-rofi for keyboard-driven restore",
      "History survives restarts",
      "Records which window you copied from (Hyprland, sway or xdotool)",
      "Blacklist apps, like password managers, so their copies are never stored",
      "Pause and resume capture from the CLI",
    ],
    wayland:
      "Watches the clipboard with wl-paste (data-control: sway, Hyprland, KDE, …). Source window from Hyprland or sway.",
    requirements: [
      "Linux desktop with GTK",
      "A system tray (eww, waybar, KDE, …)",
      "rofi (optional, for tyboard-rofi)",
      "hyprctl, sway or xdotool (optional, to record the source window)",
    ],
    usage: `tyboard run                 # start the daemon
tyboard history [--json]    # print history
tyboard pause-capture
tyboard resume-capture
tyboard capture-status      # running | paused
tyboard-rofi                # pick an entry and restore it`,
    config: `clipboard:
  backend: auto        # auto | gtk | wl_clipboard
  active_window:
    backend:
      kind: auto        # auto | disabled | command
    blacklist:
      - keepassxc`,
    nixSettings: `clipboard = {
  backend = "auto";
  active_window = {
    backend.kind = "auto";
    blacklist = [ "keepassxc" ];
  };
};`,
  },
  {
    name: "tykey",
    tagline: "Text expansion in every app.",
    summary:
      "Type a short trigger in any window and it's replaced with your text. Expansions can " +
      "include dates, emoji, command output and key presses.",
    features: [
      "Triggers work in any window",
      "Expansions defined in YAML, reloaded on change if you want",
      "Macros: {{KEY:ENTER}}, {{SLEEP_MS:100}}, {{MOVE_CARET:-5}}",
      "Templates: {{DATE}}, {{TIME}}, {{CMD:...}}, {{EMOJI:rocket}}",
      "Your own reusable globals, like a sign-off",
      "Tray menu of snippets that copy to the clipboard",
      "Optional notifications on expansion",
    ],
    wayland:
      "Reads keyboards from /dev/input (join the input group) and types with wtype. Needs the virtual-keyboard protocol (sway, Hyprland, …).",
    requirements: [
      "X11, or a Wayland compositor with virtual-keyboard",
      "On Wayland: the input group and wtype",
      "A system tray (optional, for the snippets menu)",
    ],
    usage: `tykey run                 # start listening
tykey validate-config
tykey status --follow --json   # for bars
tykey -c /path/to/config.yaml run`,
    config: `match_behavior: immediate   # immediate | boundary
watch: true
globals:
  SIGNOFF: "Thanks,{{KEY:ENTER}}Ty"
expansions:
  - trigger: "sig;"
    expansion: "{{SIGNOFF}}"
  - trigger: "date;"
    expansion: "{{DATE}}"
snippets:
  - title: "Ship status"
    content: "Shipped {{EMOJI:rocket}}"`,
    nixSettings: `match_behavior = "immediate";
watch = true;
globals.SIGNOFF = "Thanks,{{KEY:ENTER}}Ty";
expansions = [
  { trigger = "sig;"; expansion = "{{SIGNOFF}}"; }
  { trigger = "date;"; expansion = "{{DATE}}"; }
];
snippets = [
  { title = "Ship status"; content = "Shipped {{EMOJI:rocket}}"; }
];`,
  },
  {
    name: "tycapture",
    tagline: "Screen recording from your tray.",
    summary:
      "Record the screen, a region, or one window at an exact size, with mic and system " +
      "audio. It uses GPU encoding when available and can zoom in where you click.",
    features: [
      "Screen: record everything",
      "Window: click a window, pick a size (1080p, 4K, square, vertical, custom). It moves to its own i3 or sway workspace at that size, then goes back when you stop.",
      "Region: drag a rectangle on a dimmed snapshot",
      "Microphone with level meter and optional voice cleanup; system audio",
      "NVENC or VAAPI hardware encoding, libx264 fallback",
      "Zoom on clicks that follows the pointer, saved as a separate copy",
      "Optional intro card with title and logo",
      "Crash-safe: captures losslessly first, compresses in the background",
      "Pauses dunst notifications while recording",
      "Live status output for polybar, eww and other bars",
    ],
    wayland:
      "Records with wf-recorder, one monitor at a time. Window recording needs sway or Hyprland (sway also moves it to its own workspace); zoom needs Hyprland.",
    requirements: [
      "X11, or a wlroots Wayland compositor (sway, Hyprland, …)",
      "Window recording: i3 on X11; sway or Hyprland on Wayland",
      "ffmpeg and PulseAudio, pactl, notify-send",
      "On Wayland: wf-recorder and grim",
      "A system tray (eww, waybar, KDE, …)",
    ],
    usage: `tycapture run                                  # start the tray applet
tycapture start [screen|window|region] [--now] # --now skips the dialog
tycapture stop
tycapture toggle [screen|window|region]
tycapture status --follow --format polybar     # for bars`,
    config: `recording:
  format: mp4         # mkv | mp4
  quality: high       # low | medium | high | lossless
  encoder: auto       # auto | software | nvenc | vaapi
audio:
  microphone: true
  system_audio: false
zoom:
  enabled: true`,
    nixSettings: `recording = {
  format = "mp4";
  quality = "high";
  encoder = "auto";
};
audio = {
  microphone = true;
  system_audio = false;
};
zoom.enabled = true;`,
  },
  {
    name: "tyshot",
    tagline: "Screenshots you can mark up.",
    summary:
      "Freeze the screen, pick a region or a window, add arrows, text or blur, then copy " +
      "or save it. It can also send the shot straight to an opencode session.",
    features: [
      "Region selection on a frozen snapshot of all monitors",
      "Click a window to select it exactly",
      "Box, arrow, pen, highlighter, text, numbered steps and blur",
      "Measure (M): drag a line for a live pixel-length label, included in exports and HiDPI aware",
      "Frames: wrap the shot in a browser, phone or window frame",
      "Copy as PNG (stays pasteable after tyshot exits) or save to ~/Pictures/tyshot",
      "Send to an opencode session, or start a new one",
      "Keyboard shortcut for every tool",
      "Full-screen capture with optional delay",
      "Optional tray applet; HiDPI aware",
    ],
    wayland:
      "Screenshots with grim; the overlay covers every monitor (layer-shell). Window snapping on sway and Hyprland.",
    requirements: [
      "X11, or a Wayland compositor with layer-shell and screencopy",
      "On Wayland: grim and wl-copy",
      "notify-send, xdg-open",
      "A system tray (optional, for the tray applet)",
    ],
    usage: `tyshot                              # select a region (default)
tyshot full [--delay 3] [--save]    # whole screen
tyshot gui --raw > shot.png         # PNG to stdout
tyshot tray                         # run the tray applet

# i3 / sway keybinding
bindsym Print exec --no-startup-id tyshot`,
    config: `save:
  output_dir: ~/Pictures/tyshot
overlay:
  color: "#e5484d"
  dim: 0.5
annotation:
  line_width: 3
  blur_radius: 10
opencode:
  submit: false`,
    nixSettings: `save.output_dir = "~/Pictures/tyshot";
overlay = {
  color = "#e5484d";
  dim = 0.5;
};
annotation = {
  line_width = 3;
  blur_radius = 10;
};
opencode.submit = false;`,
  },
  {
    name: "tyspeak",
    tagline: "Dictation that stays on your machine.",
    summary:
      "Press a key, talk, press it again, and the text is typed where your cursor is. " +
      "The speech models run locally, so your audio never leaves your computer.",
    features: [
      "Toggle or push-to-talk from a hotkey",
      "Pick a model from the tray: Parakeet, Whisper, Qwen3-ASR, SenseVoice",
      "Shared downloader with tytts: resumable .part files, validated HTTP ranges, and cross-process file locking",
      "SHA-256-verified downloads; checksum mismatches automatically retry once from scratch",
      "Daemon checks rejected models and automatically repairs corrupt files when auto_download is enabled; dictation waits until a model is ready",
      "Types into the focused window, copies to the clipboard, or both",
      "D-Bus API for scripts and bars",
      "Live status output for polybar, eww and other bars",
      "Offline transcription of WAV files",
    ],
    wayland:
      "Types with wtype and copies with wl-copy. Needs the virtual-keyboard protocol (sway, Hyprland, …).",
    requirements: [
      "X11, or a Wayland compositor with virtual-keyboard",
      "PulseAudio or PipeWire (pipewire-pulse)",
      "xdotool (X11) or wtype and wl-copy (Wayland), notify-send",
      "A system tray (eww, waybar, KDE, …)",
    ],
    usage: `tyspeak run                       # start the daemon
tyspeak toggle                    # start / stop dictation
tyspeak models                    # list models
tyspeak model parakeet-v3         # switch model
tyspeak transcribe note.wav       # offline

# i3 / sway keybinding
bindsym $mod+d exec --no-startup-id tyspeak toggle`,
    config: `model: parakeet-unified-en
audio:
  source: default
  max_duration: 300
output:
  mode: type          # type | clipboard | both | none
  trailing_space: true`,
    nixSettings: `model = "parakeet-unified-en";
audio = {
  source = "default";
  max_duration = 300;
};
output = {
  mode = "type";
  trailing_space = true;
};`,
  },
  {
    name: "tylight",
    tagline: "Warmer screen at night.",
    summary:
      "Warms your screen colors after sunset and cools them back at sunrise, like Redshift " +
      "or Gammastep. It also sets your monitors' real brightness, with smooth fades.",
    features: [
      "Color temperature follows the sun, with smooth fades",
      "Location from GeoClue2, or a fixed latitude and longitude in the config",
      "Disable for 30 minutes, 1 hour, 2 hours, or until morning",
      "Monitor brightness: laptop backlights (through logind, no root) and external monitors over DDC/CI",
      "Ambient light sensor reading in the tray",
      "Optional auto brightness from the light sensor, smoothed so it doesn't jump around",
      "Named presets of temperature and brightness",
      "A small graph of the day's temperature and brightness",
      "Works on X11 and wlroots Wayland (Sway, Hyprland, river, niri, …)",
      "Live status output for polybar, eww and other bars",
    ],
    wayland:
      "Uses wlr-gamma-control: sway, Hyprland, river, niri, labwc. Not GNOME or KDE. Monitor brightness works everywhere.",
    requirements: [
      "X11 (RandR 1.3+), or a Wayland compositor with wlr-gamma-control",
      "GeoClue2, unless you set a location",
      "ddcutil (optional, for external monitors) and access to /dev/i2c-*",
      "A system tray (optional, for the tray applet)",
    ],
    usage: `tylight run                     # daemon + tray
tylight toggle                  # on / off, with a fade
tylight disable --for 1h        # or --until-morning
tylight set 3400                # hold a temperature
tylight auto                    # follow the sun again
tylight brightness +10          # monitor brightness
tylight status --follow --format polybar

# i3 / sway keybindings
bindsym $mod+F9 exec tylight toggle
bindsym XF86MonBrightnessUp exec tylight brightness +10
bindsym XF86MonBrightnessDown exec tylight brightness -10`,
    config: `temperature:
  day: 6500          # kelvin; 6500 is neutral
  night: 4000
# location:          # set this to skip GeoClue
#   latitude: 39.10
#   longitude: -84.51
fade_seconds: 2
backend: auto        # auto | x11 | wayland
monitor_brightness:
  backlight: true
  ddc: true`,
    nixSettings: `temperature = {
  day = 6500;
  night = 4000;
};
# Set a location to skip GeoClue:
# location = { latitude = 39.10; longitude = -84.51; };
fade_seconds = 2;
backend = "auto";
monitor_brightness = {
  backlight = true;
  ddc = true;
};`,
  },
  {
    name: "tygesture",
    tagline: "Run commands with touchpad gestures.",
    summary:
      "Maps touchpad swipes, pinches, twists and holds to shell commands, like fusuma or " +
      "libinput-gestures. It reads libinput directly, so gestures can also work like sliders.",
    features: [
      "Swipe, pinch, rotate and hold, per finger count",
      "Repeat bindings that fire while you move: a swipe as a volume slider, a twist as a knob",
      "Reads gestures straight from libinput, not debug-events text",
      "A live window that draws each gesture against its threshold",
      "Tray icon that flashes on each gesture, with pause and resume",
      "Works on X11 and Wayland",
    ],
    wayland:
      "Works the same: it reads libinput directly. Bind gestures to wtype and swaymsg or hyprctl instead of xdotool and i3-msg.",
    requirements: [
      "Read access to /dev/input/event* (the input group)",
      "A system tray (optional, for the tray applet and window)",
    ],
    usage: `tygesture run                   # daemon + tray
tygesture debug                 # print gestures as you make them
tygesture check                 # validate the config, list bindings
tygesture show                  # open the gesture window
tygesture status --follow --json  # for bars`,
    config: `swipe:
  3:
    left: xdotool key alt+Right
    right: xdotool key alt+Left
    up: { command: "pactl set-sink-volume @DEFAULT_SINK@ +5%", repeat: true }
  4:
    left: i3-msg workspace prev
    right: i3-msg workspace next
pinch:
  2: { in: xdotool key ctrl+minus, out: xdotool key ctrl+plus }`,
    nixSettings: `swipe = {
  "3" = {
    left = "xdotool key alt+Right";
    right = "xdotool key alt+Left";
    up = { command = "pactl set-sink-volume @DEFAULT_SINK@ +5%"; repeat = true; };
  };
  "4" = {
    left = "i3-msg workspace prev";
    right = "i3-msg workspace next";
  };
};
pinch."2" = {
  "in" = "xdotool key ctrl+minus";
  out = "xdotool key ctrl+plus";
};`,
  },
  {
    name: "typort",
    tagline: "Dev servers in your tray.",
    summary:
      "Lists the ports your user is listening on (Vite, Next.js, http.server, …) in the tray. " +
      "Open one in the browser, copy its URL, or stop it.",
    features: [
      "One menu entry per port, with the page title, the server (vite, next) and its project folder",
      "Click the URL to open it, copy it, stop or force kill",
      "Shows whether a port is reachable from other machines",
      "Reads /proc directly: no root, no lsof",
      "CLI and live status output for bars",
    ],
    wayland:
      "Works the same: it only needs a tray.",
    requirements: ["Linux", "A system tray (eww, waybar, KDE, …)"],
    usage: `typort run                 # tray applet
typort list                # your listening ports
typort open 5173           # in the browser
typort kill 5173           # stop it (--force for SIGKILL)
typort status --follow --format polybar`,
    config: `ignore_ports: [631]
ignore_processes: [kdeconnectd]
https_ports: [8443]
browser: [firefox, --new-tab]   # default xdg-open`,
    nixSettings: `ignore_ports = [ 631 ];
ignore_processes = [ "kdeconnectd" ];
https_ports = [ 8443 ];
browser = [ "firefox" "--new-tab" ];`,
  },
  {
    name: "tyci",
    tagline: "GitHub Actions and pull requests in your tray.",
    summary:
      "Shows running and recently finished GitHub Actions runs, plus " +
      "recently opened pull requests. Click one to open it. Uses the gh CLI you're already logged in to.",
    features: [
      "Running and recently finished CI runs across your repos",
      "Recently opened PRs from any GitHub search (involves:@me by default)",
      "Icon turns yellow while CI runs and red when a run fails",
      "Notification when a run finishes, click to open it",
      "Whitelist and blacklist repos, with owner and * globs",
      "Live status output for polybar, eww and other bars",
    ],
    wayland:
      "Works the same: it only needs a tray.",
    requirements: [
      "gh, logged in",
      "notify-send, xdg-open",
      "A system tray (eww, waybar, KDE, …)",
    ],
    usage: `tyci run                                # tray applet
tyci list [--json]                      # fetch once and print
tyci status --follow --format polybar   # for bars`,
    config: `poll_seconds: 60
repos:
  include: [tylergets, my-org/api-*]   # whitelist (empty: all)
  exclude: [tylergets/old-thing]       # blacklist
pull_requests:
  query: "involves:@me"
  max_age_days: 14
notifications:
  on_finish: true`,
    nixSettings: `poll_seconds = 60;
repos = {
  include = [ "tylergets" "my-org/api-*" ];
  exclude = [ "tylergets/old-thing" ];
};
pull_requests = {
  query = "involves:@me";
  max_age_days = 14;
};
notifications.on_finish = true;`,
  },
  {
    name: "typk",
    tagline: "The password prompt for admin actions.",
    summary:
      "A polkit authentication agent: when pkexec, a disk mount or a system setting needs " +
      "your password or fingerprint, typk asks for it in a small, focused window.",
    features: [
      "Password or fingerprint, whatever your PAM setup for polkit asks",
      "Fingerprint readers get a breathing fingerprint shared with tylock, with the password behind a button",
      "typk verify: scripts and AI agents ask you to confirm, and only see the exit code",
      "The asking app is the headline, with its icon and a quiet Kill button",
      "Optionally darkens the screen behind the prompt",
      "Shows PAM messages and allows a few attempts",
      "Account chooser when several users may authenticate",
      "Action id and details on request",
      "Always on top and focused, then out of the way",
    ],
    wayland:
      "The prompt is a layer-shell surface on the overlay layer with the keyboard, so it works on sway, Hyprland, river and KDE.",
    requirements: [
      "polkit and systemd-logind (or elogind)",
      "No other polkit agent in the session",
      "pam_fprintd in the polkit-1 PAM service, for fingerprints (optional)",
    ],
    usage: `typk run                                 # the agent
typk verify --reason "Deploy to prod?"   # exit 0 once you confirmed
typk validate-config`,
    config: `max_attempts: 3
show_details: false
dim_screen: false`,
    nixSettings: `max_attempts = 3;
show_details = false;
dim_screen = false;`,
  },
  {
    name: "tylock",
    tagline: "A screen locker with password and fingerprint support.",
    summary:
      "A screen locker for X11 and Wayland: your wallpaper with a large clock, unlocked " +
      "with your password or fingerprint through PAM. It renders at each monitor's resolution, including 1080p and 4K.",
    features: [
      "Fingerprint and password side by side: touch the reader or start typing",
      "The same breathing fingerprint animation as typk",
      "Unlocks through PAM, with the reader's and PAM's own messages",
      "Your wallpaper, dimmed and optionally blurred, on every monitor",
      "Drawn at each monitor's real resolution, the layout scaled to its size",
      "Caps Lock warning and a hidden cursor",
      "Keeps Dunst, tynotif, menus, and other popups behind the lock screen, including composited X11 notifications",
      "Works with xss-lock (holds sleep until locked) and swayidle (--daemonize)",
      "Show a quote from tyquote or any command, without delaying locking or authentication",
    ],
    wayland:
      "Locks with ext-session-lock-v1 (sway, Hyprland, river, niri, KDE): the compositor keeps the session locked even if tylock crashes.",
    requirements: [
      "PAM services tylock and, for fingers, tylock-fingerprint",
      "fprintd with enrolled prints, for fingerprints (optional)",
    ],
    usage: `tylock                  # lock until unlocked
tylock --daemonize      # return once locked (swayidle before-sleep)
tylock validate-config`,
    config: `wallpaper: ~/.background-image
blur: 0
dim: 0.3
time_format: "%H:%M"
fingerprint: true
quote_command: [tyquote, today]  # optional; install tyquote too`,
    nixSettings: `wallpaper = "~/.background-image";
blur = 0;
dim = 0.3;
time_format = "%H:%M";
fingerprint = true;
# Optional: enable programs.tyquote too (see its page).
quote_command = [ "tyquote" "today" ];`,
  },
  {
    name: "tyquote",
    tagline: "Your quotes, on every machine.",
    summary:
      "A personal quote collection stored as plain JSON files. Add a thought, pick one at random, " +
      "or show today's quote on your lock screen. Sync the folder with Syncthing; no daemon needed.",
    features: [
      "Add quotes with optional authors, from an argument or stdin",
      "Random picks, the latest addition, or a consistent quote for today",
      "Same collection and local date, same daily quote on every machine",
      "One independently named JSON file per quote, with atomic writes",
      "Sync concurrent additions with Syncthing, without a shared database",
      "Plain-text output for tylock and other scripts",
      "Configurable storage, default author, attribution, and local or UTC daily rollover",
    ],
    requirements: ["Linux", "Syncthing (optional, for syncing between machines)"],
    usage: `tyquote add "Keep going." --author "A reminder"
tyquote add - --author "Author" < quote.txt
tyquote random
tyquote latest
tyquote today
tyquote --directory ~/Quotes today
tyquote --config ~/other-quotes.yaml today
tyquote validate-config`,
    config: `directory: ~/.local/share/tyquote/quotes
default_author: null
show_author: true
author_prefix: "— "
day_boundary: utc   # local (default) | utc`,
    nixSettings: `directory = "~/.local/share/tyquote/quotes";
default_author = null;
show_author = true;
author_prefix = "— ";
day_boundary = "utc";`,
    integration: `imports = [
  inputs.tys-tools.homeManagerModules.tyquote
  inputs.tys-tools.homeManagerModules.tylock
];
programs.tyquote.enable = true;
programs.tylock = {
  enable = true;
  settings.quote_command = [
    "\${config.programs.tyquote.package}/bin/tyquote" "today"
  ];
};`,
    storage:
      "Quotes live in $XDG_DATA_HOME/tyquote/quotes (normally ~/.local/share/tyquote/quotes). " +
      "Sync that folder with Syncthing and ignore .tyquote-*.tmp. Edit or delete the JSON files " +
      "to change your collection. Each file contains text, an optional author, and created_at. " +
      "Use directory in your config or --directory to choose another folder. " +
      "Syncing additions or deletions can change today's selection. Set day_boundary to utc on " +
      "every machine for a shared midnight, or use local (the default) to follow each machine's date.",
  },
];

export const findTool = (name) => tools.find((t) => t.name === name);

export const nixConfig = (tool) => tool.nixConfig ??
  `imports = [ inputs.tys-tools.homeManagerModules.${tool.name} ];
programs.${tool.name} = {
  enable = true;
  settings = {
${tool.nixSettings.split("\n").map((line) => `    ${line}`).join("\n")}
  };
};`;
