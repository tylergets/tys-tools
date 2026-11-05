# tyboard

`tyboard` is a small Linux clipboard daemon with a tray menu and persistent history.

It watches clipboard changes, stores recent entries on disk, and lets you restore old entries from the tray icon menu.

## Features

- Clipboard history for text and images
- Tray icon menu with clickable history items
- `rofi` picker binary for quick keyboard-driven restore
- Persistent history cache across restarts
- Optional active-window metadata capture
- Single-instance lock to avoid duplicate daemons

## Requirements

- Linux desktop session (GTK/AppIndicator)
- `rofi` (optional, required for `tyboard-rofi`)
- Nix (recommended), or a Rust toolchain plus GTK/AppIndicator dev libraries

## Quick Start (Nix)

```bash
nix develop
cargo run -p tyboard -- run
```

If no config is found, create one in the project root as `tyboard.yaml` (or use the config paths described below).

## Configuration

Config is YAML.

Discovery order:

1. `./tyboard.yaml` (current working directory)
2. `~/.config/tyboard/config.yaml`

You can always override with `--config /path/to/config.yaml`.

Example:

```yaml
clipboard:
  backend: auto
  active_window:
    backend:
      kind: auto
    blacklist:
      - keepassxc
      - slack
```

### Clipboard Backend

- `auto` (default) - `gtk` on X11, `wl_clipboard` on Wayland
- `gtk` - GTK's clipboard (X11)
- `wl_clipboard` - `wl-paste --watch` (Wayland compositors with the data-control protocol:
  sway, Hyprland, KDE, …). GTK can't read the Wayland clipboard from a tray daemon.

### Active Window Backend

- `kind: auto` - tries `hyprctl` first, then (on Wayland) sway's IPC, then `xdotool`
- `kind: disabled` - do not capture active-window metadata
- `kind: command` - run a custom command and parse stdout as a title

When available, `source_window` metadata includes more than `title` and `app_id`.
`hyprctl` provides the richest details (`initial_app_id`, `initial_title`,
`window_id`, `pid`, `workspace_id`, `workspace_name`, and `is_xwayland`), sway's IPC
includes `window_id`, `pid`, `workspace_name`, and `is_xwayland`, while `xdotool` includes
`window_id`, `pid`, and `workspace_id`.

### Active Window Blacklist

Use `clipboard.active_window.blacklist` to skip clipboard capture when the focused
window matches a blocked value.

- For backends that provide an app id/class (for example `hyprctl`), values match
  app ids exactly (case-insensitive).
- If app id is unavailable, values are matched as case-insensitive substrings
  against the focused window title.

Command example:

```yaml
clipboard:
  active_window:
    backend:
      kind: command
      program: xdotool
      args:
        - getactivewindow
        - getwindowname
```

## CLI

```bash
# Run daemon (default command)
tyboard run

# Print history
tyboard history

# Print history as JSON
tyboard history --json

# Include image pixel bytes in history output
tyboard history --json --images

# Pause clipboard capture (daemon keeps running)
tyboard pause-capture

# Resume clipboard capture
tyboard resume-capture

# Print capture status (running|paused)
tyboard capture-status

# Validate config and exit
tyboard validate-config

# Use explicit config path
tyboard --config /path/to/config.yaml run

# Open clipboard history in rofi and restore selection
tyboard-rofi

# Customize rofi invocation details
tyboard-rofi --prompt "clipboard" --lines 25 --rofi-bin rofi
```

## Data and Runtime Paths

- History cache: `~/.cache/tyboard/history.json` (or platform cache dir equivalent)
- Instance lock socket: `${XDG_RUNTIME_DIR}/tyboard-<user>.sock` (falls back to temp dir)

## Home Manager Module

Part of the [tys-tools](../../README.md) flake:

```nix
{inputs, ...}: {
  imports = [inputs.tys-tools.homeManagerModules.tyboard];
  programs.tyboard = {
    enable = true;
    settings = { /* same keys as the YAML config above */ };
  };
}
```

Options: `enable`, `package` (defaults to `pkgs.tyboard` with the tys-tools overlay), `settings`
(written to `~/.config/tyboard/config.yaml`), and `service.enable` (systemd user service, default
on).

## Development

From the repo root:

```sh
nix develop
cargo test -p tyboard
cargo run -p tyboard -- --help
nix build .#tyboard
```
