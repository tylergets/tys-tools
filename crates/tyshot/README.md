# tyshot

`tyshot` is a Flameshot-style screenshot tool for X11: freeze the screen, drag out a region,
and copy it to the clipboard (or save it as PNG). It has an optional tray icon and works
well bound to a hotkey in i3.

## Features

- **Region selection** on a frozen snapshot of all monitors, with everything outside the
  selection dimmed and the size shown in pixels
- **Window snapping**: hover a window to highlight it, click to select it (including its
  window-manager decorations)
- Move the selection by dragging inside it; resize it from any edge or corner handle
- **Toolbar** beside the selection (button size configurable): select, **box**, **arrow**,
  **pen**, **highlighter**, **text**, **numbered steps**, **blur**, **frame**, undo, redo,
  **send to opencode**, **copy**, **save**, close
- **Frames**: put the screenshot in a browser window, phone or app window frame. The frame is
  shown on screen and added *around* the selection, so the copied/saved image grows to
  include it and nothing is cropped (rounded corners are transparent)
- **Send to opencode**: attach the screenshot to an [opencode](https://opencode.ai) session,
  picked by title, or start a new one in a kitty window (see [Sending to opencode](#sending-to-opencode))
- **Copy to clipboard** as `image/png`. Copied images stay pasteable after `tyshot` exits
  (see [Clipboard](#clipboard))
- **Save** to `~/Pictures/tyshot/tyshot-<YYYY-MM-DD_HH-MM-SS>.png` (configurable)
- Full-screen capture without the overlay, optional delay
- Tray applet (libappindicator): Capture Region (also on middle-click, if your tray host
  forwards it), Capture Screen, Open Screenshots Folder. The icon re-registers if the tray
  host (bar) starts after tyshot or restarts
- Desktop notification after copying or saving
- HiDPI aware: crops are exact device pixels

## Overlay controls

| Input | Action |
| --- | --- |
| Drag | Select a region (dragging outside an existing selection starts a new one) |
| Click | Select the window under the pointer (highlighted while hovering) |
| Drag inside / on an edge or handle | Move / resize the selection |
| V / R / A / B | Tool: select / box / arrow / blur (then drag on the selection to draw) |
| P / H | Tool: pen / highlighter (freehand) |
| T | Tool: text. Click to place, type; Shift+Enter new line, Enter or a click finishes, Esc discards |
| N | Tool: numbered steps. Click to place ①, ②, ③, … (drag to position) |
| F | Cycle the frame: none → browser → mobile → window |
| Ctrl+Z | Undo the last annotation |
| Ctrl+Shift+Z, Ctrl+Y | Redo |
| Arrow keys | Move the selection 1 px |
| Shift + arrow keys | Resize from the bottom-right corner 1 px |
| Ctrl+A | Select the whole screen |
| Enter, Ctrl+C, double-click | Copy to the clipboard and close (double-click: select tool only) |
| Ctrl+S | Save to the output directory and close |
| Ctrl+Enter | Send to an opencode session (pick it with 1–9, or the arrows + Enter) |
| Esc, right-click | Cancel |

Enter/Ctrl+C/Ctrl+S with no selection act on the whole screen. Every key action also has a
toolbar button; hover a button to see its shortcut. Annotations are clipped to the selection,
and the selection can still be moved or resized after drawing (switch back to the select
tool). While typing text, keys go to the text; Ctrl shortcuts finish it first.

## Requirements

- X11 session
- `notify-send` for notifications, `xdg-open` for "Open Screenshots Folder"
- For the tray: an AppIndicator/StatusNotifierItem tray host (eww `systray`, waybar, KDE, …)

The Nix package wraps the binary with these on `PATH`.

## Quick Start (Nix)

```bash
nix run .#tyshot -- gui
```

## CLI

```bash
tyshot [gui] [--delay SECONDS] [--raw]          # select a region (default command)
tyshot full [--delay SECONDS] [--save | --raw]  # whole screen; copies unless --save/--raw
tyshot tray                             # run the tray applet
tyshot sessions                         # list recent opencode sessions (id, time, dir, title)
tyshot validate-config
```

`--raw` writes the PNG to stdout instead of copying or saving it (no notification), for
scripts: `tyshot gui --raw > shot.png`. Saving prints the written path to stdout. The exit status is `0` on success, `1` if the
selection was cancelled and `2` on errors.

i3:

```
bindsym Print exec --no-startup-id tyshot
bindsym Shift+Print exec --no-startup-id tyshot full
```

## Sending to opencode

The terminal button in the toolbar (or Ctrl+Enter) sends the screenshot straight into an
[opencode](https://opencode.ai) session, through opencode's local background service (the
one its terminal UI connects to; the URL and password are read from
`~/.local/state/opencode/service.json`):

1. A list opens: the 6 most recently active sessions, with each one's title, directory and
   last activity (and whether the agent is working in it right now), then **+ New session**
   in up to 3 of their most recent directories.
2. The chosen session gets the screenshot as an image attachment. No file is written. A new
   session is created first, then opened in a new terminal window
   (`kitty --directory <dir> opencode --session <id> <dir>`) so you can talk to it there.
3. By default the agent isn't started: the screenshot waits in the session and goes to the
   model together with the next message you type there, so you can follow it with a
   question. Set `opencode.submit: true` to start the agent on it right away instead.

The service has to be running: it starts with opencode, or run `opencode service start`.
Sessions of opencode instances started with `--standalone` (private servers) aren't listed.

## Clipboard

X11 clipboard contents are served by the program that copied them. When you copy, `tyshot`
hands the PNG to a small detached `tyshot serve-clipboard` process and returns immediately:

- If a clipboard manager that supports `CLIPBOARD_MANAGER` / `SAVE_TARGETS` is running,
  the image is handed to it and the helper exits.
- Otherwise the helper keeps serving the image until another program takes the clipboard,
  then exits.

## Configuration

Config is YAML. Discovery order: `./tyshot.yaml`, then `~/.config/tyshot/config.yaml`;
override with `--config PATH`. All keys are optional.

```yaml
save:
  output_dir: ~/Pictures/tyshot   # default: XDG pictures dir + /tyshot
  base_name: tyshot               # files are <base_name>-<timestamp>.png
notifications:
  enabled: true                    # notify-send after copying/saving
overlay:
  color: "#e5484d"                 # selection outline and handles
  dim: 0.5                         # shade over the unselected area, 0–1
annotation:
  color: "#e5484d"                 # boxes, arrows, pen, text and step markers
  line_width: 3                    # box/arrow/pen thickness in pixels, 1–20
  highlight_color: "#ffd60a"       # highlighter (multiplied into the image like a marker)
  font_size: 20                    # text size in pixels, 8–96; highlighter and steps scale with it
  blur_radius: 10                  # blur strength in pixels, 1–100 (see below)
toolbar:
  button_size: 44                  # pixels, 24–96; the toolbar and icons scale with it
opencode:
  submit: false                    # true: start the agent on the screenshot immediately
  terminal: kitty                  # opens new sessions; takes kitty's --directory syntax
```

The blur is meant to hide things, not just soften them: the area is first averaged into
blocks `3 × blur_radius` pixels wide (30 px by default), which destroys detail irreversibly,
and then blurred so it looks smooth. Raise `blur_radius` if very large text should be
unrecognizable too.

## Home Manager Module

Part of the [tys-tools](../../README.md) flake:

```nix
{inputs, ...}: {
  imports = [inputs.tys-tools.homeManagerModules.tyshot];
  programs.tyshot = {
    enable = true;
    settings = { /* same keys as the YAML config above */ };
  };
}
```

Options: `enable`, `package` (defaults to `pkgs.tyshot` with the tys-tools overlay), `settings`
(written to `~/.config/tyshot/config.yaml`), and `service.enable` (systemd user service, default
on).

## Development

From the repo root:

```sh
nix develop
cargo test -p tyshot
cargo run -p tyshot -- --help
nix build .#tyshot
```
