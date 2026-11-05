// Every tool on the site. The home page, nav and tool pages are all built from this list.
// Keep text short; full details live in each crate's README.

export const version = "0.3.0";
export const repo = "https://github.com/tylergets/tys-tools";

export const tools = [
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
      "Records which window you copied from (hyprctl or xdotool)",
      "Blacklist apps, like password managers, so their copies are never stored",
      "Pause and resume capture from the CLI",
    ],
    requirements: [
      "Linux desktop with GTK",
      "A system tray (eww, waybar, KDE, …)",
      "rofi (optional, for tyboard-rofi)",
      "hyprctl or xdotool (optional, to record the source window)",
    ],
    usage: `tyboard run                 # start the daemon
tyboard history [--json]    # print history
tyboard pause-capture
tyboard resume-capture
tyboard capture-status      # running | paused
tyboard-rofi                # pick an entry and restore it`,
    config: `clipboard:
  backend: gtk
  active_window:
    backend:
      kind: auto        # auto | disabled | command
    blacklist:
      - keepassxc`,
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
    requirements: ["X11 session", "A system tray (optional, for the snippets menu)"],
    usage: `tykey run                 # start listening
tykey validate-config
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
  },
  {
    name: "tycapture",
    tagline: "Screen recording from your tray.",
    summary:
      "Record the screen, a region, or one window at an exact size, with mic and system " +
      "audio. Encodes on the GPU when it can and can zoom in where you click.",
    features: [
      "Screen: record everything",
      "Window: click a window, pick a size (1080p, 4K, square, vertical, custom). It moves to its own i3 workspace at that size, then goes back when you stop.",
      "Region: drag a rectangle on a dimmed snapshot",
      "Microphone with level meter and optional voice cleanup; system audio",
      "NVENC or VAAPI hardware encoding, libx264 fallback",
      "Zoom on clicks that follows the pointer, saved as a separate copy",
      "Optional intro card with title and logo",
      "Crash-safe: captures losslessly first, compresses in the background",
      "Pauses dunst notifications while recording",
      "Live status output for polybar, eww and other bars",
    ],
    requirements: [
      "X11 session",
      "i3 (only for window recording)",
      "ffmpeg with x11grab and PulseAudio, pactl, notify-send",
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
      "Frames: wrap the shot in a browser, phone or window frame",
      "Copy as PNG (stays pasteable after tyshot exits) or save to ~/Pictures/tyshot",
      "Send to an opencode session, or start a new one",
      "Keyboard shortcut for every tool",
      "Full-screen capture with optional delay",
      "Optional tray applet; HiDPI aware",
    ],
    requirements: [
      "X11 session",
      "notify-send, xdg-open",
      "A system tray (optional, for the tray applet)",
    ],
    usage: `tyshot                              # select a region (default)
tyshot full [--delay 3] [--save]    # whole screen
tyshot gui --raw > shot.png         # PNG to stdout
tyshot tray                         # run the tray applet

# i3 keybinding
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
      "Resumable, checksum-verified model downloads",
      "Types into the focused window, copies to the clipboard, or both",
      "D-Bus API for scripts and bars",
      "Live status output for polybar, eww and other bars",
      "Offline transcription of WAV files",
    ],
    requirements: [
      "X11 session",
      "PulseAudio or PipeWire (pipewire-pulse)",
      "xdotool, notify-send",
      "A system tray (eww, waybar, KDE, …)",
    ],
    usage: `tyspeak run                       # start the daemon
tyspeak toggle                    # start / stop dictation
tyspeak models                    # list models
tyspeak model parakeet-v3         # switch model
tyspeak transcribe note.wav       # offline

# i3 keybinding
bindsym $mod+d exec --no-startup-id tyspeak toggle`,
    config: `model: parakeet-unified-en
audio:
  source: default
  max_duration: 300
output:
  mode: type          # type | clipboard | both | none
  trailing_space: true`,
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

# i3 keybindings
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
  },
];

export const findTool = (name) => tools.find((t) => t.name === name);
